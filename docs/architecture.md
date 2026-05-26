# Architecture

This document is for maintainers and contributors. It explains how the code is organized and where to extend behavior.

## Runtime Shape

One binary has four main personalities:

- `proxy`: local unified SOCKS5H/HTTP proxy listener plus bridge manager.
- `reverse`: remote SOCKS5H listener; this machine is the TCP/UDP egress side.
- `node daemon`: symmetric service daemon. It can expose a framed transport, own proxy profile tasks, accept control commands, and report state to peers.
- `remote`: compatibility framed helper; executes TCP/UDP opens from the target host.
- `daemon`: legacy local control kernel that owns multiple proxy profile tasks.
- `host` / `service` / `control`: CLI management commands for remote helpers, local services, and daemon IPC.
- `route`: high-level route intent command. The CLI submits intent to the local node daemon; the daemon owns peer bootstrap, route planning, and route persistence.

The normal data path is:

```text
SOCKS5H or HTTP proxy client
  -> local unified ingress parser
  -> bridge frames
  -> direct QUIC, direct TLS/TCP, explicit plain TCP, SSH direct-tcpip to persistent node transport, or SSH exec stdio
  -> remote node daemon
  -> target TCP/UDP destination
```

Reverse mode swaps the SOCKS listener and egress:

```text
remote SOCKS client
  -> remote helper SOCKS5H listener
  -> bridge frames over SSH stdio
  -> local reverse transport
  -> destination from the local machine
```

## Source Layout

- `src/main.rs`
  - Thin binary entrypoint only.
  - Owns the global allocator, logging initialization, config loading, and top-level CLI dispatch.
  - Runtime behavior belongs in explicit `src/*.rs` modules.

- `src/cli.rs`
  - Clap/serde command and argument definitions.
  - Shared by the thin CLI entrypoint, daemon control requests, route planning, and persisted route specs.

- `src/ssh_client.rs`
  - `russh` session setup, OpenSSH-style target parsing, ProxyJump connection chaining, host-key
    checks, SSH exec, upload, and direct-tcpip streams.
  - Authentication itself lives in `src/ssh_auth.rs`.

- `src/deploy.rs`
  - SSH bootstrap and remote host management orchestration.
  - Connects through `ssh_client`, applies config defaults, records peer install metadata, and
    dispatches host management commands.
  - Delegates detailed behavior to focused submodules:
    - `src/deploy/remote_commands.rs`: remote shell command builders for config writing,
      systemd/nohup service management, node control forwarding, logs, clean, and doctor.
    - `src/deploy/helper.rs`: helper probing, sidecar/current-binary selection, upload, and helper
      exec command construction.
    - `src/deploy/transport.rs`: direct QUIC, TLS/TCP, plain TCP, SSH direct-tcpip, SSH exec, and
      `auto` peer transport opener.

- `src/bridge.rs`
  - Local bridge manager for the framed `SPX1` proxy stream.
  - Presents per-flow TCP/UDP handles to ingress code and maps them to `protocol::Frame`.

- `src/controller.rs`
  - Legacy local proxy/control kernel and bridge reconnect loop.
  - Kept as an implementation detail for `proxy` and compatibility commands while new workflows
    move toward `node daemon` + `route`.

- `src/socks.rs`
  - Unified SOCKS5H and HTTP proxy ingress parser.
  - Converts client TCP/UDP proxy requests into bridge operations.

- `src/remote.rs`
  - SSH exec fallback/helper runtime.
  - Runs stdio framed transport, direct TCP framed transport, or a remote reverse SOCKS listener.

- `src/protocol.rs`
  - Compact framed protocol shared by normal proxy and reverse proxy.
  - Defines `Frame`, `UdpDatagram`, frame size limits, binary encode/decode, and protocol unit tests.

- `src/peer_transport.rs`
  - Transport selection model for the daemon-to-daemon data plane.
  - Defines the target fallback order: QUIC, TLS-over-TCP, optional plain TCP, SSH direct-tcpip, SSH exec.
  - The current implemented auto path tries configured QUIC, configured TLS/TCP, optional plain TCP when explicitly allowed, then SSH direct-tcpip to daemon TCP transport, then SSH exec helper fallback.

- `src/control_socket.rs`
  - Cross-OS daemon control IPC.
  - Supports TCP, Unix domain sockets, and Windows named pipes.
  - Applies shared request/response byte limits and 30-second IO timeouts so control clients cannot
    hold daemon tasks or memory indefinitely.

- `src/route.rs`
  - High-level route intent planner.
  - Builds route intent JSON for the CLI and exposes pure planning helpers used by `node_daemon`.
  - Expands saved target profiles, chooses route owner by direction, validates reachable local peer addresses for remote-owned routes, and emits daemon route JSON.

- `src/config.rs`
  - Local config management.
  - Owns `config init/show/profiles`, `profile-set/profile-remove`, `token`, and `cert-import`.
  - Persists node identity, peer registry records, SSH auth references, known_hosts policy, ProxyJump chains, peer transport defaults, saved remote tokens, and TLS/mTLS certificate paths.

- `src/config/diagnostics.rs`
  - Redacted config diagnostics and offline descriptor exchange.
  - Owns `config inspect`, `config export-descriptor`, and `config import-descriptor` materialization
    so config presentation and peer adoption logic do not keep growing the core config model file.

- `src/ssh_auth.rs`
  - SSH authentication strategy for `russh` sessions.
  - Tries SSH agent/Pageant/OpenSSH agent first, then configured or default unencrypted identity files, then `none`.
  - Keeps private key material outside `~/.ssh_proxy/config.toml`; profiles store only paths and policy references.

- `src/node_daemon.rs`
  - Symmetric node service shell.
  - Owns daemon startup/shutdown, shared `NodeManager` state, status/link JSON, and legacy profile
    connect/disconnect.
  - Delegates node behavior to focused submodules:
    - `src/node_daemon/control_client.rs`: CLI-side `node control` request construction.
    - `src/node_daemon/control_protocol.rs`: typed node control request model and JSON line
      materialization shared by the CLI, route planner, SSH-mediated remote host control, reporter,
      and control server. New JSON requests carry `api_version = 1`; legacy text commands and older
      JSON without a version remain accepted during the current reshaping phase.
    - `src/node_daemon/control_server.rs`: daemon control socket listener, request parsing, and
      dispatch.
    - `src/node_daemon/args.rs`: route/control CLI argument materialization.
    - `src/node_daemon/transport.rs`: plain TCP, TLS/TCP, and QUIC peer transport listeners.
    - `src/node_daemon/routes.rs`: daemon-owned route specs, persistent route store, route
      start/stop/restart/restore, and route supervision.
    - `src/node_daemon/peers.rs`: peer registry output, peer bootstrap/forget, peer status reports,
      route-intent planning, and bootstrap-before-route behavior.

- `src/service.rs`
  - Local service command orchestration.
  - `service print/install/uninstall/start/stop/status` now delegates shared planning and
    platform execution to focused submodules.
  - `service install` copies the current executable into a stable user install directory by
    default, writes missing daemon defaults to `~/.ssh_proxy/config.toml`, generates a secure
    transport token, auto-selects an available user transport port, then points the service at
    `ssh_proxy node daemon`.
  - `service status` now prints a redacted project-level JSON summary before delegating to the
    platform status command. It includes config/route-store/binary paths, selected endpoints,
    token/cert presence, report targets, config schema health, route-store version/duplicate-ID
    checks, saved peer endpoint diagnostics, and a best-effort daemon status query.

- `src/service/plan.rs`
  - Service install plan and daemon command materialization.
  - Resolves user/system scope, stable install path, daemon control endpoint, peer transport
    listeners, token/config materialization, TLS/mTLS paths, and binary copy behavior.

- `src/service/platform.rs`
  - OS-specific service execution.
  - Linux: systemd user/system.
  - macOS: launchd user/system.
  - Windows: schtasks for user scope, sc.exe for system scope.

- `src/sidecar.rs`
  - Embedded Linux musl helper extraction.
  - Used when a Windows/macOS binary needs to upload a Linux helper.

- `src/logging.rs`
  - Tracing subscriber initialization.

- `build.rs`
  - Enforces Linux musl sidecar availability for every non-musl build.
  - Copies the helper into `OUT_DIR/linux-musl-sidecar.bin`, then `sidecar.rs` embeds it with `include_bytes!`.

## Build Contract

The build contract is intentionally strict:

```text
cargo zigbuild --target x86_64-unknown-linux-musl
cargo build
```

The first command builds the Linux helper. The second command embeds that helper into the current OS binary.

For release, build the musl release helper before building the host release binary. `SSH_PROXY_LINUX_MUSL_BIN` can override the sidecar path.

Compile-time choices:

- `tokio` is feature-scoped to `macros`, `rt-multi-thread`, `net`, `io-util`, `io-std`, `sync`, `time`, and `fs`. Avoid returning to `full` unless a new subsystem genuinely needs the extra APIs.
- `profile.dev` and `profile.test` keep incremental builds on, reduce debug info, and use many codegen units. This improves local iteration while release keeps `opt-level = 3`, fat LTO, one codegen unit, aborting panics, and stripping.
- `scripts/cargo-sccache.ps1` and `scripts/cargo-sccache.sh` opt into `sccache` without making it mandatory. Do not hard-code `rustc-wrapper = "sccache"` in `.cargo/config.toml` unless the deployment environment guarantees that binary exists.

## SSH Layer

`ssh_client` uses `russh` directly:

- Resolves OpenSSH-ish target syntax: `host`, `user@host`, `host:port`.
- Reads `~/.ssh/config` for HostName, User, Port, IdentityFile, UserKnownHostsFile, StrictHostKeyChecking, and ProxyJump.
- Implements ProxyJump with SSH `direct-tcpip`; no OpenSSH subprocess is required for the data path.
- Auth order: agent first, then identity files, then `none`.
- Windows agent failures are non-fatal; the code falls through to identity files.

Known limitation: encrypted private keys require an agent.

Profile/auth storage:

- `~/.ssh_proxy/config.toml` carries `schema_version = 1`. Missing schema versions are accepted as
  legacy v1-compatible configs; future schema versions are rejected with an upgrade hint instead of
  being interpreted silently.
- Every node now has an `[identity]` block with `node_id`, `node_name`, and `secret`. `config init` and `service install` generate it automatically.
- Peer nodes are tracked under `[peers.<alias>]`. A peer record stores the peer node identity, target alias, selected control endpoint, selected peer transport, token, trust source, and last-seen timestamp.
- Daemon and peer tokens can carry redacted `TokenMetadata`: creation time, rotation time, scope,
  and optional expiry. Old configs with only `token = "..."` still load; metadata is populated when
  a token is generated, rotated, or recorded through bootstrap/refresh.
- `config inspect` is the redacted configuration view. It exposes schema, endpoint, identity,
  profile, peer, token metadata, certificate path, and trust-source presence without printing node
  secrets, daemon tokens, profile remote tokens, or peer tokens.
- `config export-descriptor` and `config import-descriptor` support offline or one-way adoption.
  Exported descriptors are redacted and contain endpoints, protocol versions/features, auth
  presence, and token metadata but not token values. Import can attach an out-of-band token and
  records the peer/profile with `trust = "descriptor-import"` by default.
- SSH bootstrap records every configured remote peer listener it knows about: plain TCP,
  TLS-over-TCP, and QUIC. Profiles mirror the same endpoints so later route planning can prefer
  private transports without repeating install-time flags.
- Peer records also maintain a `transport_protocols` summary ordered by preferred data-plane
  attempt order: `quic`, `tls-tcp`, then `plain-tcp`. Older configs without that field derive the
  same list from stored endpoints when displayed.
- SSH bootstrap is considered a trust event. When `host <target> start` succeeds, the local config records the remote peer and the remote config records the local node as `peers.bootstrap-local`.
- Profiles store references to SSH identity files, not private key material. This keeps OpenSSH-compatible key management outside the config.
- Agent auth is runtime state and is never persisted; the profile records only the target/user/port/jump/known_hosts values needed to resolve the connection.
- `accept_new` can be persisted per profile, but should be treated as a bootstrap convenience rather than a long-term strict host-key policy.

Certificate storage:

- `config cert-import` copies TLS and mTLS files into `~/.ssh_proxy/certs/<name>/`.
- Remote verification material attaches to a profile with `remote_ca`, `remote_client_cert`, and `remote_client_key`.
- Local listener material attaches to daemon config with `tls_cert`, `tls_key`, and `tls_client_ca`.
- On Unix, imported key files are chmod `0600`; cert/CA files are chmod `0644`.

## Bridge Protocol

`protocol::Frame` is a compact framed protocol over any async read/write stream:

- `OpenTcp`
- `OpenTcpResult`
- `Data`
- `Close`
- `UdpPacket`
- `Log`

The local side multiplexes accepted proxy flows over one bridge. SOCKS5H and HTTP `CONNECT` become TCP streams; HTTP absolute-form requests are rewritten to origin-form and then sent as TCP data. SOCKS UDP associate maps to framed UDP datagrams. The remote side maps frames to outbound `TcpStream` and `UdpSocket` operations.

Frame IO is bounded by `MAX_FRAME` (16 MiB). Writers reject oversized payloads before emitting a
header, readers reject oversized length headers, and structured frame decoders reject trailing bytes
so protocol drift or corrupted payloads fail loudly instead of being silently accepted.

## Peer Transport Roadmap

The application-layer protocol should remain one framed route/data/control protocol. Transport selection sits below it.

Preferred order for new environments:

1. `quic`: implemented primary data plane. It gives UDP-native behavior, QUIC transport recovery, lower head-of-line blocking at the transport layer, and better adaptation to lossy networks. Current runtime uses one bidirectional QUIC stream carrying the existing `SPX1 + Frame` protocol.
2. `tls-tcp`: implemented conservative fallback for networks that block QUIC but allow normal TCP. It uses rustls with pinned roots and optional mTLS.
3. `plain-tcp`: implemented local-only or explicitly trusted environments. It stays opt-in because it lacks transport security.
4. `ssh-direct`: current stable fallback through SSH `direct-tcpip` into the remote daemon transport.
5. `ssh-exec`: bootstrap/fallback helper path when no daemon transport is reachable.

`remote_transport=auto` currently tries the implemented stable subset: configured QUIC first, configured TLS/TCP second, explicit plain TCP third when `allow_plain_tcp` is true, SSH direct-tcpip to the remote daemon transport fourth, then SSH exec helper. The module is deliberately test-driven so QUIC stream-per-flow and QUIC mTLS can be added without changing route semantics.

Implemented daemon transport handshake:

1. Optional shared token prefix, retained for current deployments.
2. `PeerHello` JSON packet with magic `SPX1`, version, node name, candidate protocol, and feature list.
3. `PeerWelcome` JSON packet with accepted protocol and remote feature list.
4. Existing `protocol::Frame` stream.

The handshake is intentionally below route semantics and above the physical stream. TLS/TCP, SSH direct-tcpip, and future QUIC should all converge on the same hello/welcome capability exchange before data frames begin.

Direct Peer Transport Status:

- `node daemon --quic-transport --tls-cert --tls-key` exposes a direct QUIC peer listener.
- `proxy --remote-transport quic --remote-quic --remote-ca --remote-name` opens a direct QUIC daemon stream without establishing SSH.
- `tokio-rustls` is added with the `ring` backend, not aws-lc.
- `peer_transport` can build client/server TLS configs from PEM material.
- `node daemon --tls-transport --tls-cert --tls-key` exposes a direct TLS peer listener.
- `node daemon --tls-client-ca` requires connecting TLS peers to present a client certificate rooted in that PEM bundle.
- `proxy --remote-transport tls-tcp --remote-tls --remote-ca --remote-name` opens a direct daemon stream without establishing SSH.
- `proxy --remote-client-cert --remote-client-key` presents a client certificate for mTLS.
- `proxy --remote-transport plain-tcp --remote-tcp` opens the daemon's normal TCP transport directly without SSH. This is insecure unless the network is already trusted.
- `auto` uses QUIC first when `--remote-quic` is configured, TLS/TCP next when `--remote-tls` is configured, can use plain TCP only with `--allow-plain-tcp`, then falls back to SSH transports.
- Tests prove explicit QUIC and `auto` QUIC can carry normal SOCKS traffic without the SSH client path.
- Tests prove explicit TLS/TCP and `auto` TLS/TCP can carry normal SOCKS traffic without the SSH client path.
- Tests prove mTLS accepts clients with a trusted certificate and rejects clients that omit it.
- Tests prove explicit plain TCP and `auto + allow_plain_tcp` can carry normal SOCKS traffic without the SSH client path, including the optional transport token prefix.

## Reconnect and Recovery

`controller::run_bridge_manager` owns the SSH bridge lifecycle:

- Tracks attempts, success/failure counters, active/total TCP counts, generation, and last error.
- Uses exponential backoff with `reconnect_delay_secs` and `reconnect_max_delay_secs`.
- Wraps bridge connect in `connect_timeout_secs`.
- On bridge loss, clears the active bridge handle so new SOCKS requests wait for reconnect or fail if reconnect is disabled.

`reverse` uses the same timeout and exponential backoff shape, but its remote SOCKS listener is tied to the SSH exec session. If the SSH session drops, local `reverse` reconnects and starts a fresh remote SOCKS listener.

Before starting reverse mode, `deploy` probes an explicit remote helper path in `auto` mode. The probe verifies that the helper supports `remote --reverse-socks` and checks the requested remote listen port with `ss` when available. This catches stale helpers and common port conflicts before the framed stream starts.

The node control API exposes status, descriptor, shutdown, connect/disconnect profile operations,
route start/stop, peer bootstrap/registry management, transport/link counters, and peer reports.
The legacy daemon control API remains for compatibility.

Node control requests now have a single typed construction path:

- local `node control` commands;
- short `route` intent requests;
- `host <target> node-*` commands tunneled through SSH to the remote node CLI;
- daemon peer status reports.

`node control descriptor` returns the local node's peer descriptor: node identity, binary version,
OS/arch, control API version, peer protocol version, advertised protocol features, control/data
endpoints, transport protocol summary, token/certificate presence, route store path, and autostart
setting. It is intentionally redacted: token values and private key paths are not exposed.
`node control token-rotate` rotates the daemon control/transport token after authenticating the
current request, updates the daemon's in-memory token and config file, and returns the new token plus
metadata to the caller. Dual-token grace windows are still future work.
`node control peer-rotate-token <target>` performs the remote half through SSH: it calls the peer's
`token-rotate`, parses the new remote token, refreshes the descriptor when possible, and records the
updated token/metadata in the local profile and peer registry.

`node control peer-refresh <target>` is the first adoption path. The local daemon connects through
SSH, runs the remote node's `descriptor` command, and records the returned identity/endpoints
without uploading a new binary or restarting the service. `route` uses the same refresh attempt
before bootstrap when no usable peer record exists, so an already-running remote daemon can be
adopted into the local registry instead of being overwritten.

`node control peer-diff <target>` uses the same SSH descriptor query but does not mutate local
configuration. It returns a redacted comparison of the saved profile/peer record against the remote
descriptor, including node identity, control/data endpoints, transport protocol set, token presence
and token metadata scope. The response includes `changed` and `next_action` so the CLI can point the
operator toward `peer-refresh` for stale descriptors or `peer-rotate-token` for token-only drift.

`node control peer-check-version <target>` is the explicit compatibility diagnostic. It queries the
remote descriptor over SSH and compares the remote control API version, peer data protocol version,
advertised features, and package version with the local binary. A remote control API newer than the
local binary or a future peer data protocol reports `next_action = "upgrade-local"`. Missing,
older, or feature-incomplete peer protocol metadata reports `next_action = "peer-bootstrap --force"`.
Package-version differences are advisory when protocol checks still pass.

The server rejects requests with a future `api_version`, while still accepting missing versions for
older bootstrap helpers. Responses now preserve their existing top-level fields while adding
`api_version = 1`; common success/error replies use the shared response envelope and include stable
error `code` values. Rich status/list responses still expose domain fields at the top level for CLI
and script ergonomics.

Control IPC has bounded IO. Requests larger than 1 MiB are rejected with `bad_request`, responses
larger than 16 MiB are rejected by the client, and connect/read/write operations time out after 30
seconds. These limits are intentionally generous for route and peer descriptors while preventing
accidental unbounded reads.

TCP control endpoints are token-protected when the daemon has a configured token. Local
`node control` injects `--token` when provided, otherwise it falls back to `[daemon].token` from the
local config. Unix sockets and Windows named pipes continue to rely on their user-scoped OS
boundary. This keeps local service installs ergonomic while preventing an unauthenticated localhost
TCP control socket from accepting lifecycle commands.

SSH-mediated `host <target> node-*` commands also pass the recorded remote token explicitly to the
remote `node control` CLI. This keeps remote management working even when the remote user's home or
config discovery differs between interactive SSH sessions, systemd user services, and nohup
supervisors.

Route task semantics:

- `forward`: local node binds a local SOCKS5H/HTTP listener and connects through an SSH target. With `remote_transport=tcp`, it uses the remote node daemon transport.
- `reverse`: local node owns the task and SSH session, while the target binds the remote SOCKS5H listener. Traffic egresses from the local node.
- Route IDs are unique within a node daemon. Starting a duplicate ID fails.
- Forward routes preflight the local listen address to catch port conflicts before spawning; the route table also rejects duplicate local listener ownership.
- Reverse routes reject duplicate `target + remote_listen` ownership inside one daemon. Remote OS port conflicts are still surfaced through the route task's SSH/session logs.
- Route starts are persistent by default. The daemon writes durable route specs to `~/.ssh_proxy/routes.json` unless the route was created with `--volatile`.
- Route store writes use the same private temp-file replacement path as config writes, avoiding
  partially written JSON after a process crash or disk error.
- Route-start success responses include the route ID, owner, direction, listener, peer, detail, and
  persistence fields so daemon-first commands can render useful output without scraping route
  tables.
- Startup restores persistent routes by default. `node daemon --no-route-autostart` disables that restore pass.
- Each route has an internal supervisor. Task exit/error updates route stats (`state`, `attempts`, `restart_count`, `last_error`, `last_event`) and then restarts with capped backoff.
- `stop-route` aborts the task and removes it from the route store. `restart-route` aborts and respawns from the in-memory spec, then rewrites the route store if the route is persistent.
- `routes` returns route state without unrelated profile/transport detail; `status` and `links` include the same route array.
- `host node-forward`, `host node-reverse`, `host node-routes`, `host node-restart-route`, and `host node-stop-route` send the same route JSON to a remote installed node daemon through SSH. This keeps service-installed operation daemon-owned even when the management path is SSH.

## Remote Host Management

Remote node management lives in `deploy` for now. It should eventually move to `src/remote_host.rs`.

Supported remote management commands:

- `start`
- `status`
- `node-status`
- `node-links`
- `node-descriptor`
- `node-forward`
- `node-reverse`
- `node-routes`
- `node-restart-route`
- `node-stop-route`
- `node-connect`
- `node-disconnect`
- `logs`
- `doctor`
- `restart`
- `stop`
- `clean`

Persistence:

- `auto`: try systemd user; fallback to nohup supervisor.
- `systemd`: user-level systemd service that runs `node daemon`; best-effort `loginctl enable-linger` for logout/reboot survival on Linux.
- `nohup`: small supervisor script plus pidfile, child pidfile, and logfile under `~/.ssh_proxy`; runs `node daemon` and restarts it with capped backoff.

Persistent remote installs default to a user software path. The resolver prefers a directory already present on the remote `PATH` (`~/.local/bin`, `~/bin`, then `~/.ssh_proxy/bin`), otherwise creates `~/.local/bin/ssh_proxy`. This keeps the node CLI discoverable for interactive remote management without requiring root.

Control endpoints are user-scoped by default to avoid multi-user collisions:

- Windows: named pipe includes the username.
- Linux: `$XDG_RUNTIME_DIR/ssh_proxy.sock`, falling back to `~/.ssh_proxy/control.sock`.
- macOS: `~/.ssh_proxy/control.sock`, falling back to `/tmp/ssh_proxy-<user>.sock`.

Local `service install` enables a per-user localhost transport by default. It is derived from the username to reduce collisions on shared machines; if that port is unavailable during first install, the planner scans the next 200 ports and saves the selected value. Operators can pin a stable transport with `service --transport <addr> install`, disable it with `service --no-transport install`, or add `transport_listen` under `[daemon]` in `~/.ssh_proxy/config.toml`.

The daemon now treats config as a first-class source for transport listeners. If CLI flags omit them, `node daemon` reads `transport_listen`, `tls_transport_listen`, `quic_transport_listen`, `tls_cert`, `tls_key`, `tls_client_ca`, `token`, and `report_to` from the local config. This keeps installed services short while preserving explicit CLI overrides for tests and one-off runs.

Persistent remote installs run a small discovery script before service creation. The script:

- creates `~/.ssh_proxy`;
- generates or receives the shared token from the local controller;
- preserves an existing remote `node_id` and `node_name` when re-bootstrap updates a node;
- scans for available control and transport ports near the requested defaults;
- writes the selected values into the remote `~/.ssh_proxy/config.toml`;
- returns the selected ports to the local installer so the systemd/nohup command matches the recorded config.

`route` command behavior:

- The CLI never creates the route directly. It sends `{"cmd":"route_intent","route":...}` to the local node daemon.
- Before planning, the daemon checks `[peers.<target>]`. If the peer lacks usable control/transport metadata, the daemon uses russh to upload or update the remote node daemon, writes remote config, starts persistence, and saves the returned node identity/endpoints locally.
- Before falling back to install/update, the daemon first tries descriptor refresh over SSH. If the
  remote already has a working `ssh_proxy node daemon` and remote CLI config can authenticate to
  its local control endpoint, the local node adopts the descriptor and records the peer without
  reinstalling.
- `node control peer-bootstrap <target>` is the same SSH bootstrap/update path without creating a route. It is useful for provisioning, token refresh, and explicit remote binary upgrades.
- `node control peer-refresh <target>` is the non-installing descriptor refresh/adoption path.
- `route <target> --direction local-uses-remote --port <listen-port>` becomes a persistent `forward` route owned by the local daemon. The local daemon owns the listener, and the egress side is the remote node.
- `route <target> --direction remote-uses-local --port <listen-port> --connect-mode auto` first tries the symmetric direct plan when `--local-peer` or a non-loopback local daemon transport is available. That direct plan asks the remote daemon to own a persistent `forward` listener, and the egress side is this node through the supplied reachable peer transport address.
- If direct peer transport is unavailable in `auto`, the planner submits a persistent `reverse` route to the local daemon. The local daemon owns a long-lived data tunnel initiated from the local side, the remote side owns the SOCKS5H/HTTP listener, and proxy flows return over that established tunnel. This handles the same NAT shape as SSH reverse tunneling without reusing the control connection for data.
- For private/local peer addresses, the route planner permits plain `SPX1` TCP because the network is assumed trusted. Public routes should use TLS/TCP or QUIC with certificates.
- `--connect-mode direct` keeps the old strict behavior and fails early when no reachable local peer exists.
- `--connect-mode reverse-link` skips direct peer planning and always creates the local-initiated reverse route.
- Every high-level `route_intent` response now includes a structured `plan` object. The shape is shared across local-forward, remote-direct, and reverse-link plans: route ID, owner, mode, listener, egress, transport candidates, selected transport, fallback reason, next action, and persistence.

The `doctor` command is intended for troubleshooting container-like targets. It reports user, uid, home, pid 1, systemd availability, helper binary status, pidfile/logfile status, and listening socket state.

## Testing Notes

Local:

```text
cargo fmt -- --check
cargo check
cargo test
```

Musl sidecar:

```text
cargo zigbuild --target x86_64-unknown-linux-musl
cargo build
```
