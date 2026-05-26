# Operations

This document describes public operational behavior without environment-specific
test records, host aliases, ports, or private network details.

## One Binary Flow

`ssh_proxy` is one executable with several roles:

- CLI for service, host, route, and config operations.
- Local daemon for control, route supervision, and peer transports.
- Remote daemon installed through SSH bootstrap.
- Proxy ingress for SOCKS5H, HTTP CONNECT, and fixed TCP tunnel listeners.

Typical setup:

```powershell
ssh_proxy service --scope user install
ssh_proxy host <remote-host> --accept-new --persist auto start
ssh_proxy route <remote-host> --direction local-uses-remote --port <local-proxy-port>
```

For the opposite direction:

```powershell
ssh_proxy route <remote-host> `
  --direction remote-uses-local `
  --port <remote-proxy-port> `
  --connect-mode reverse-link
```

The `route` command sends an intent to the local daemon. The daemon checks peer
records, refreshes or bootstraps the remote daemon when needed, selects a
transport, starts the route owner, and records persistent routes unless
`--volatile` is used.

## Installing Services

Install the local user daemon:

```powershell
ssh_proxy service --scope user install
ssh_proxy service status
```

Install or refresh a remote daemon:

```powershell
ssh_proxy host <remote-host> --accept-new --persist auto start
ssh_proxy host <remote-host> node-status
```

`--persist auto` tries a user service manager first and falls back to a user
supervisor when the platform lacks a service manager. Installs prefer user-level
locations and avoid requiring root privileges.

## Route Directions

`local-uses-remote` binds a listener on this machine and opens outbound targets
from the remote node:

```text
local application -> local listener -> remote node -> target
```

`remote-uses-local` binds a listener on the remote node and opens outbound
targets from this machine:

```text
remote application -> remote listener -> local node -> target
```

When the remote node can connect back to a local peer transport, use
`--connect-mode direct --local-peer <host>:<port>`. When it cannot, use
`--connect-mode reverse-link`; the local daemon keeps a long-lived connection to
the remote side and carries remote flows back through that connection.

## Transport Selection

`remote_transport = "auto"` chooses among configured transports:

1. QUIC when configured and reachable.
2. TLS/TCP when configured and reachable.
3. Explicitly allowed plain TCP for trusted or lab networks.
4. SSH `direct-tcpip` to a remote daemon transport.
5. SSH exec helper fallback.

Route planning explains the decision chain:

```powershell
ssh_proxy route <remote-host> `
  --direction local-uses-remote `
  --port <local-proxy-port> `
  --explain
```

Status output includes selected protocol, route owner, pool size, active
workers, bytes, open failures, fallback reason, degraded reason, and
protocol-specific health fields.

## SSH Fallback Modes

`ssh-native` uses russh `direct-tcpip` channels directly for simple proxy flows.
It is the lowest-overhead SSH fallback when the remote daemon does not need to
own route policy or restore state.

`spx-over-ssh-direct` uses SSH `direct-tcpip` to reach a remote daemon transport
and keeps SPX framing in the data plane. Use it when remote daemon policy,
tokens, route restore, UDP behavior, or daemon-owned status must stay involved.

`ssh-exec` remains an emergency compatibility path for bootstrap and restricted
targets.

## Peer Records and Authentication

Each daemon has a node identity and local config under the user's application
data directory. Peer records store redacted identity metadata, endpoint
descriptors, transport protocol support, trust source, token metadata, and
certificate references.

Tokens and private keys are not printed by status or descriptor commands.
Runtime SSH authentication can use an SSH authentication agent or configured key
files, but private key material remains outside `ssh_proxy` config.

Certificate material imported through `config cert-import` is copied into the
project's certificate store with private permissions for key files where the
platform supports them.

## Existing Remote Daemons

When a target already has a daemon but the local machine has no peer record, use
descriptor refresh before reinstalling:

```powershell
ssh_proxy node control peer-refresh <remote-host> --accept-new
```

Use `peer-diff` to inspect redacted drift without mutating local config:

```powershell
ssh_proxy node control peer-diff <remote-host> --accept-new
```

Use `peer-bootstrap --force` only when the remote binary or service should be
repaired or upgraded.

## Version Compatibility

Daemon descriptors include package version, control API version, peer data
protocol version, and feature bits. Version checks report the safest next action
without silently overwriting remote state:

```powershell
ssh_proxy node control peer-check-version <remote-host> --accept-new
```

A future remote control API usually means the local binary should be upgraded.
A missing or incompatible peer data protocol usually means the remote should be
bootstrapped with the current binary.

## Token Rotation

Rotate a local daemon token:

```powershell
ssh_proxy node control token-rotate
```

Rotate a saved peer token through SSH:

```powershell
ssh_proxy node control peer-rotate-token <remote-host>
```

Offline descriptor exchange can be used when operators need an explicit
out-of-band approval step:

```powershell
ssh_proxy config export-descriptor
ssh_proxy config import-descriptor <remote-host> descriptor.json --token <out-of-band-token>
```

## Multi-User Instances

User-scope installs keep control endpoints, config paths, tokens, routes, and
service definitions scoped to the current OS user. System-scope installs should
be reserved for environments where a single shared daemon is intentional.

On shared machines, prefer user-scope daemons and explicit peer endpoints. Avoid
copying route stores or token files between users.

## Cleanup

Stop routes through the daemon that owns them:

```powershell
ssh_proxy node control stop-route <route-id>
ssh_proxy host <remote-host> node-stop-route <route-id>
```

Remove a remote user install only when you own that install:

```powershell
ssh_proxy host <remote-host> clean
```

Cleanup commands should remove only `ssh_proxy` resources they created. They
must not delete unrelated user services, unrelated listeners, or shared peer
records without operator intent.

## Troubleshooting

Useful commands:

```powershell
ssh_proxy service status
ssh_proxy node control status
ssh_proxy node control routes
ssh_proxy host <remote-host> doctor
ssh_proxy host <remote-host> logs --lines <line-count>
```

## Local Run Configuration

Concrete lab targets, upstream proxy URLs, private ports, key paths, and token
values should live outside Git. Benchmark scripts load `scripts/bench.local.ps1`
when it exists, then read environment variables such as
`SSH_PROXY_BENCH_TARGETS`, `SSH_PROXY_BENCH_UPSTREAM_PROXY`,
`SSH_PROXY_BENCH_URL`, and `SSH_PROXY_BENCH_READINESS_URL`.

Use `scripts/bench.local.example.ps1` as the template and keep the edited
`scripts/bench.local.ps1` uncommitted.

When reporting issues, redact:

- host aliases and private DNS names;
- private IP addresses and concrete port assignments;
- daemon tokens and peer tokens;
- certificate private keys;
- SSH private key paths;
- proxy credentials.
