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
- `doctor_collect`

Jobs move through `queued`, `running`, `waiting_retry`, `healthy`, `failed`, and
`cancelled`. Each job records `phase`, `progress`, `blocker`, `next_action`,
structured `repair_action`, `last_error`, retry timing, recovery attempts,
timestamps, and recent events.

## Proxy Sessions

`ssh_proxy up` and `ssh_proxy vscode up` create an `ensure_proxy_session` job.
The daemon state machine is:

```text
resolve_target -> validate_local_proxy -> select_remote_port -> ensure_peer
  -> ensure_transport -> start_route -> wait_route_ready
  -> verify_remote_port -> apply_remote_settings -> health_monitoring -> healthy
```

The command returns accepted state quickly. The daemon continues work in the
background and clients poll `status` or `events`.

## Peer And Transport Model

`ensure_peer` uses this order:

1. Adopt an existing compatible peer descriptor.
2. Schedule a remote peer update if the version is old.
3. Bootstrap with Rust SSH when no descriptor is readable.
4. Report `requires_external_ssh=true` only when Rust SSH lacks a required SSH capability.

Normal transport preference is:

1. Existing direct daemon transport.
2. Rust reverse-link route.
3. Rust SSH `direct-tcpip`.
4. Rust SSH exec helper for constrained bootstrap.
5. Explicit emergency external SSH only when the daemon reports why it is required.

OpenSSH subprocess fallback is not part of the normal VS Code path.

## Remote Setup

Remote setup is daemon-owned. The daemon applies and repairs:

- VS Code Machine `http.proxy` and `http.proxySupport`;
- terminal proxy environment;
- `~/.vscode-server/server-env-setup`;
- remote Git proxy config;
- `~/.vscode-server/remote-proxy-status.json`.

The VS Code extension calls `ssh_proxy vscode apply-settings` rather than
running remote setup scripts itself. VS Code Machine settings are read and
rendered by Rust, then written through a minimal SSH shell file operation; remote
`node` is diagnostic-only and is not required for the normal settings path.

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
