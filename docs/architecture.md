# Architecture

`ssh_proxy` v0.3 uses a Docker-like daemon model. The daemon is the only
production control plane. CLI commands and the VS Code extension are thin
clients that submit allowlisted intents and render daemon state.

## Control Plane

```text
VS Code extension / CLI
  -> private named pipe or Unix socket
  -> ssh_proxy daemon
  -> job engine, session store, peer registry, route runtime, remote setup
```

Windows production uses a system daemon with a private named pipe. Linux and
macOS use a daemon with a private Unix socket. TCP control is legacy or explicit
development mode and requires token authentication.

The daemon owns these persistent stores:

- `daemon_state.json`: daemon identity, schema version, update state.
- `jobs.json`: job snapshots plus recent event summaries.
- `sessions.json`: workspace, target, route, remote URL, ownership, health.
- `peers.json`: peer descriptors, versions, transports, last-seen state.
- `routes.json`: route specs, readiness, transport, and metrics.

State writes use atomic temp-file plus rename. Startup reconciliation adopts
known sessions and routes, refreshes peers, and resumes unfinished jobs.

## Protocol Layers

v0.3 keeps existing wire formats stable while sharing the protocol vocabulary:

- `protocol_core::version` owns control API version, peer protocol version,
  feature sets, and compatibility classification.
- `protocol_core::envelope` owns `ControlEnvelope`, `ControlResponse`, and
  `ControlError`. Daemon JSON-line control responses are generated through this
  shape while keeping existing public fields such as `api_version`, `ok`,
  `code`, `message`, `error`, and `data`.
- `protocol_core::control` owns daemon command alias normalization and typed
  dispatch names. Legacy wide `NodeRequest` JSON still parses, but command
  matching does not live in the socket server.
- `protocol_core::descriptor` owns the typed peer descriptor DTO used by config
  import/export, descriptor refresh, compatibility checks, remote install
  endpoint adoption, and doctor output. Subsystems should parse descriptors once
  into `PeerDescriptor` instead of re-reading endpoint and protocol fields from
  raw JSON.
- `protocol_core::codec` wraps the SPX data frame codec and the generic
  `magic + version + length + JSON` control-frame shell used by QUIC-native
  control. The SPX 9-byte header and the `QNC1` outer frame remain unchanged.
- `protocol_core::report` owns shared health state, dependency
  classification, repair-action references, and runtime decision DTOs.
  Daemon-owned reports such as peer descriptors, route status, and route
  readiness should materialize typed report structs first, then serialize to the
  legacy-compatible public JSON shape.
- `protocol_core::redaction` delegates to the lifecycle redaction rules so
  token, credential, identity path, and `known_hosts` handling stays consistent
  across status, doctor, daemon events, and VS Code diagnose.

The protocol boundary is intentionally split by layer:

- Control plane: JSON-line daemon control and remote peer control use shared
  envelopes, version checks, command names, error fields, blockers, and repair
  actions.
- Data plane: SPX frames keep their binary format and are addressed through the
  shared `DataFrame`/`SpxFrameCodec` contract.
- Native QUIC control: `QNC1` still frames JSON route-control messages, but the
  framing helper is shared with other control-plane transports.
- Reports: status, doctor, events, and VS Code diagnose should render shared
  DTOs instead of rebuilding health, dependency, connection, and redaction
  fields in each subsystem.
  Route runtime output keeps the legacy transport fields and `decision_chain`,
  but also publishes the shared `connection_decision` DTO so daemon status,
  doctor reports, and UI rendering can consume one decision surface.

OpenSSH subprocess compatibility and remote shell TCP probes are not part of
the normal protocol stack. They remain explicit compatibility or diagnostic
paths and must surface `requires_external_ssh`, dependency classification, and a
repair action when used.

## Workspace Layers

The workspace uses horizontal core crates plus one vertical application crate.
Dependencies should flow upward only:

```text
core
  <- protocol
  <- lifecycle / config / control / ssh / transport / route / deploy / service / daemon
  <- cli command contracts and app adapters in crates/ssh-proxy
  <- binary bootstrap
```

The first extraction pass keeps compatibility shims in `crates/ssh-proxy/src`
so existing module paths can migrate gradually. New shared DTOs, codecs,
lifecycle models, control socket helpers, SSH primitives, and CLI command
contracts should live in the appropriate workspace crate rather than expanding
the binary crate. `ssh-proxy-route` owns route decision and route status DTOs;
`ssh-proxy-deploy` owns remote install and remote setup artifact intents;
`ssh-proxy-service` owns service-management contracts; `ssh-proxy-daemon` owns
command-neutral daemon/job/session/peer/state DTOs. `ssh-proxy-cli` is a
command-contract crate; command dispatch still lives in the app crate until the
remaining runtime orchestration is promoted. Lower crates must not depend on
app, CLI dispatch, SSH executors, or platform service runtimes unless the
dependency direction is explicitly promoted in a separate architecture change.

## Symmetric Peer Lifecycle

Local daemons and remote peer servers share the same lifecycle vocabulary:

```text
prepare -> inspect_descriptor -> dependency_check -> stage_binary
  -> write_config -> install_service -> start_service -> health_probe
  -> record -> healthy | repairing | rollback | failed
```

The implementation keeps platform differences behind small adapters:

- `PeerLifecycleSpec` is the shared model for `local_daemon` and `remote_peer`
  roles; legacy CLI/daemon entrypoints convert into this model before reporting
  service state.
- `PeerLifecycleContext` carries the current role, platform, scope, provider,
  executor, store, and report sink. Entrypoints should pass this context across
  same-layer helpers instead of threading loosely related arguments.
- `LocalExecutor` and `SshExecutor` run the same lifecycle against local files
  or Rust SSH exec/upload/direct-tcpip.
- `LifecyclePlan` contains executable actions (`StageBinary`, `WriteArtifact`,
  `ReadArtifact`, `RunCommand`, `ProbeTcp`, `ServiceControl`) so daemon jobs and
  remote peer bootstrap use one execution path instead of parallel scripts.
- `LifecycleEventSink` streams phase reports while work is running; job records,
  peer status, events, and doctor output are derived from the same report stream.
- Service providers render platform operations for Windows SCM, Windows user
  scheduled tasks, systemd, launchd, and the managed nohup supervisor.
- Remote install no longer executes service-manager commands as an ad hoc SSH
  call. `install_remote` builds a `PeerLifecycleSpec(remote_peer)` and runs the
  provider command plan through `SshExecutor` and the lifecycle workflow, so
  command failure, phase reporting, and redaction use the same path as daemon
  jobs.
- Rust materializes peer `config.toml`, `peer_state.json`,
  `install_report.json`, `health.json`, and `routes.json`; remote shell usage is
  limited to minimal file writes and platform service commands.
- `peer_lifecycle::store` validates and redacts peer state, install report,
  health, and routes documents before they are written or surfaced in reports.
- `PeerLifecycleReport` is reused by local service install reports and remote
  peer status so doctor/status output has the same state, phase, provider,
  blocker, retry, and recovery vocabulary.

`service-manager` is useful as an interface reference, but the production path
keeps the existing `windows-service` + elevated worker transaction until a
separate compatibility pass proves it can preserve versioned binaries, UAC
logging, rollback behavior, and remote provider command rendering.
See `docs/service-provider-evaluation.md` for the provider contract and adoption
gate.

## Component Boundaries

The v0.3 lifecycle code is the execution authority. Other subsystems translate
user intent into lifecycle specs, route specs, or setup artifacts and then hand
execution to shared modules.

- `peer_lifecycle` owns shared specs, lifecycle actions, event sinks, executor
  traits, service-provider contracts, connection decision metadata, stores,
  reports, and redaction. It should not know VS Code command text or daemon RPC
  framing.
- `service` owns local CLI option parsing and platform permission boundaries.
  `service.rs` is the thin command entrypoint; `service::report`,
  `service::status`, `service::health`, and `service::labels` own install
  reports, daemon status JSON, local health probes, and user-visible labels.
  It builds `PeerLifecycleSpec(local_daemon)` and calls the lifecycle runner.
  Platform behavior is split behind provider adapters for systemd, launchd,
  Windows SCM, and Windows user scheduled tasks; Windows SCM FFI, UAC worker
  behavior, versioned ProgramData binaries, and rollback stay in the Windows
  SCM adapter.
- `deploy` owns remote bootstrap inputs, descriptor refresh, token/config
  materialization, and compatibility helpers. Remote peer installation runs as
  `PeerLifecycleSpec(remote_peer)` through `SshExecutor`.
- `node_daemon::remote_peer` owns daemon RPC/job glue, retry/adopt policy, and
  peer registry updates. It streams lifecycle events instead of rebuilding
  install phase reports. The public RPC wrapper stays in `remote_peer.rs`;
  `remote_peer::job_runner` sequences descriptor adoption, install fallback,
  and terminal failure recording; `remote_peer::report` and `phase_mapping`
  own status DTOs and lifecycle/job phase conversion.
- `node_daemon::peers` owns peer command dispatch and peer registry operations.
  Route intent orchestration lives in `peers::route_intent`, which builds route
  plans, ensures peer prerequisites, and preserves the daemon route response
  shape before returning to the control server.
- `node_daemon::proxy_session` owns the session state machine that sequences
  remote peer ensure, route creation, Rust-native handoff, remote setup, and
  health monitoring. `ProxySessionSpec`, SSH target details, apply policy, and
  URL/key helpers live in the `spec` submodule so the runner consumes a stable
  intent model. Status rendering helpers live in the `status` submodule;
  `apply_settings` owns the direct VS Code apply-settings command path, and
  `route_ready` owns route readiness, handoff probing, remote setup, and final
  health transition. `job_runner` sequences these modules and owns route
  conflict repair, so `proxy_session.rs` remains the daemon RPC surface.
- `node_daemon::remote_setup` owns VS Code and shell environment artifacts. Rust
  renders payloads and uses `SshExecutor.write_artifact`; shell remains limited
  to stdin file writes, optional Git config, cleanup, and platform commands.
  `ssh-proxy-deploy::RemoteArtifactIntent` is the command-neutral place that
  names the server directory, relative path, artifact kind, backup policy, and
  read/write command shape. The app crate adapts those intents to `SshExecutor`,
  so deploy models do not depend on SSH runtime code.
- `quic_native::runtime` owns listener orchestration and data-flow accounting.
  Connection establishment lives in `runtime::connection`; status rendering
  lives in `runtime::status` with `snapshot`, `profile`, and `render`
  submodules; worker metric mutation lives in `runtime::worker_metrics`, while
  status-only worker snapshots live in `runtime::metrics_snapshot`. Control
  keepalive and stream I/O stay in their existing focused modules. `QNC1`
  control framing and stream behavior remain compatibility-owned by the
  protocol modules, not by status rendering.
- `node_daemon::management` owns daemon update transactions and the preview node
  control surface. User/report JSON for nodes, jobs, job events, and peer
  ensure/update wrappers lives in `management::report`; staged self-update
  orchestration and switch-script helpers live in `management::update`.
- `node_daemon::state` owns daemon state file orchestration. The serialized job,
  session, peer, remote setup, and daemon records live in `ssh-proxy-daemon`;
  app-side store modules keep file I/O, async locking, schema compatibility, and
  corrupt-file quarantine behavior.
- `route` owns user-visible route plans and preflight probes. Transport names,
  direct-policy labels, SSH-mode labels, and data-plane reasons come from
  `peer_lifecycle::connection` so status, doctor, daemon, and route output use
  one vocabulary. Daemon route status consumes `RouteRuntimeDecision` instead of
  rebuilding selected transport, preflight, and SSH-mode metadata. New consumers
  should prefer `connection_decision` for typed transport selection and read the
  older route metadata only for compatibility or detailed diagnostics.

## Public CLI Surface

Production commands are:

```text
ssh_proxy daemon install|uninstall|start|stop|status|update|serve
ssh_proxy up|down|status|events|doctor
ssh_proxy vscode up|status|diagnose|apply-settings
```

Older `service`, `node control`, `route`, and `host` entrypoints are hidden or
internal compatibility tools. They should not appear in user repair hints,
README workflows, or VS Code normal diagnostics.

## Job Engine

Long work is represented as daemon jobs:

- `ensure_proxy_session`
- `apply_remote_settings`
- `self_update`
- `remote_peer_update`
- `ensure_remote_peer`
- `doctor_collect`

Jobs move through `queued`, `running`, `waiting_retry`, `healthy`, `failed`, and
`cancelled`. Each job records `phase`, `progress`, `blocker`, `next_action`,
structured `repair_action`, `last_error`, retry timing, recovery attempts,
timestamps, and recent events.

## Proxy Sessions

`ssh_proxy up` and `ssh_proxy vscode up` create an `ensure_proxy_session` job.
The daemon state machine is:

```text
resolve_target -> validate_local_proxy -> select_remote_port -> ensure_remote_peer
  -> ensure_transport -> start_route -> wait_route_ready
  -> verify_remote_port -> apply_remote_settings -> health_monitoring -> healthy
```

The command returns accepted state quickly. The daemon continues work in the
background and clients poll `status` or `events`.

## Peer And Transport Model

`ensure_remote_peer` is the default path for `up` and `vscode up`. It uses this
order:

1. Adopt an existing compatible peer descriptor.
2. Inspect dependency and service-manager capability.
3. Stage the remote binary and materialize peer config/state artifacts.
4. Install and start the peer service through the shared provider command plan.
5. Health-check the remote descriptor and transport.
6. Record local `peers.json` state and continue the proxy session.

Normal transport preference is:

1. Explicit CLI or profile transport.
2. Existing reachable TLS/TCP peer transport.
3. Existing reachable QUIC peer transport.
4. Explicitly trusted plain TCP peer transport.
5. Rust SSH `direct-tcpip` to the persistent peer transport.
6. Rust reverse-link when topology requires it.
7. Explicit `ssh-exec` compatibility only when requested.
8. Explicit emergency external SSH only when the daemon reports why it is required.

OpenSSH subprocess fallback is not part of the normal VS Code path.

Route runtime metadata uses the shared transport helpers for transport names,
direct transport policy, SSH mode labels, and SSH data-plane reasons. The daemon
does not maintain a second copy of those user-facing decisions.

Remote peer service management follows the local daemon model where possible:
Linux prefers user systemd and falls back to a managed nohup supervisor; macOS
uses a LaunchAgent; Windows remotes use a user scheduled task unless an explicit
elevated compatibility path is requested.

## Remote Setup

Remote setup is daemon-owned. The daemon applies and repairs:

- VS Code Machine `http.proxy` and `http.proxySupport`;
- terminal proxy environment;
- `~/.vscode-server/server-env-setup`;
- remote Git proxy config;
- `~/.vscode-server/remote-proxy-status.json`.

The VS Code extension calls `ssh_proxy vscode apply-settings` rather than
running remote setup scripts itself. VS Code Machine settings, server-env, and
remote status JSON are read or rendered by Rust, then written through
`SshExecutor.write_artifact` with stdin-backed file operations. Remote `node` is
diagnostic-only and is not required for the normal settings path.

## Updates

Daemon self-update is an allowlisted job:

```text
stage_update -> verify_update -> switch_binary -> restart_daemon
  -> health_check -> healthy | rollback | failed
```

System daemon update requires daemon authority. Non-interactive clients return
`requires_elevation` and a concrete `next_action`; they do not trigger UAC or
sudo prompts on their own.

Remote peer update uses the same staged-copy, verify, switch, health-check, and
rollback pattern.

## VS Code Extension

The extension does only five things:

1. Detect the current Remote SSH target.
2. Detect or read the local proxy URL.
3. Call `ssh_proxy vscode up`.
4. Poll `ssh_proxy vscode status` and job events.
5. Render phase, blocker, next action, and remote URL.

It does not own service install, local leases, session daemon fallback,
OpenSSH fallback, route readiness loops, or remote setup scripts.

## Error Shape

JSON errors and blockers use:

- `blocker`
- `next_action`
- `repair_action`
- `last_error`
- `requires_elevation`
- `requires_external_ssh`
- `retry_after_ms`

Clients should display these fields directly and avoid inventing their own
fallback chain.

`ssh_proxy doctor --json --report` adds dependency classification, redacted daemon
state, recent install logs, handoff probes, route health, peer state, and remote
setup state for issue reports.

## Build Notes

The project is Rust-first and avoids C FFI unless a future plan documents why a
Rust-native option is not viable. Release builds use the configured allocator
and optimized profile. Linux musl artifacts are built through `cargo zigbuild`.
