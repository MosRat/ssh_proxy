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
