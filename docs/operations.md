# Operations

This document describes the production operating model for the current v0.3
daemon architecture. It intentionally avoids older `service`, `node control`,
session-daemon, lease, and OpenSSH fallback workflows as user-facing paths.

## Production Model

`ssh_proxy daemon serve` is the local authority. CLI commands, the VS Code
extension, and future remote peers submit allowlisted requests to the daemon over
a private named pipe or Unix socket.

The daemon owns:

- daemon install, start, stop, status, update, and health;
- proxy session jobs and event history;
- peer adoption, bootstrap, version checks, and update jobs;
- route planning, startup, readiness, and metrics;
- remote VS Code settings, server-env, Git config, status files, cleanup, and drift repair.

Normal user commands are:

```powershell
ssh_proxy daemon install --scope system --elevate
ssh_proxy daemon status --json
ssh_proxy up --target <ssh-host> --local-proxy http://127.0.0.1:10808/ --json
ssh_proxy status --workspace <workspace-id> --json
ssh_proxy events --job <job-id> --json
ssh_proxy down --workspace <workspace-id> --json
ssh_proxy doctor --json --report
```

VS Code uses the same daemon surface through:

```powershell
ssh_proxy vscode up --target <ssh-host> --workspace <workspace-id> --local-proxy <url> --json
ssh_proxy vscode status --workspace <workspace-id> --json
ssh_proxy vscode apply-settings --target <ssh-host> --workspace <workspace-id> --proxy-url <remote-url> --json
ssh_proxy vscode diagnose --workspace <workspace-id> --json
```

## Permission Behavior

Windows production installs use a system daemon. Interactive install commands
use a native elevated worker that writes a structured JSONL install log and
stages the binary under `%ProgramData%\ssh_proxy\bin\<version-hash>`. The service
ImagePath points at that versioned binary, so reinstall/update does not copy over
a running executable. Background and auto-start paths do not pop UAC. They return
structured blockers and `repair_action`:

- `requires_elevation`
- `daemon_unavailable`
- `daemon_pipe_access_denied`
- `node_control_token_required`
- `requires_external_ssh`

User-facing repair commands should point at `ssh_proxy daemon install`,
`ssh_proxy daemon update`, `ssh_proxy status`, or `ssh_proxy doctor`.

## Proxy Session Lifecycle

`up` creates an `ensure_proxy_session` job. The daemon records a session id,
route id, remote URL, and job id, then advances:

```text
resolve_target -> validate_local_proxy -> select_remote_port -> ensure_remote_peer
  -> ensure_transport -> start_route -> wait_route_ready
  -> verify_remote_port -> apply_remote_settings -> health_monitoring -> healthy
```

The initial CLI response is accepted quickly. Long work happens in the daemon job
engine. Use `status` and `events` for readiness and failure reasons.

## Remote Setup

Remote setup is daemon-owned. The VS Code extension no longer uploads or runs
remote setup scripts. The daemon writes and repairs:

- remote VS Code Machine settings;
- terminal proxy environment;
- `~/.vscode-server/server-env-setup`;
- remote Git proxy config;
- `~/.vscode-server/remote-proxy-status.json`.

`down` stops the route and applies cleanup according to the session policy.

## Health And Recovery

The daemon periodically checks:

- control socket health;
- route readiness and listener reachability;
- peer descriptor freshness;
- remote setup hash drift;
- job retry windows and terminal blockers.

If the daemon restarts, it restores `jobs.json`, `sessions.json`, `peers.json`,
and `routes.json`, quarantines corrupt state files, then reconciles unfinished
jobs instead of leaving orphaned local state.

## Target Peer Service

Remote SSH targets run a managed peer service. On Linux, the daemon prefers the
user systemd unit `ssh-proxy-helper.service`; when user systemd is unavailable,
it falls back to the managed nohup supervisor under `~/.ssh_proxy/run`.
macOS remotes use a user LaunchAgent with KeepAlive. Windows remotes use a user
scheduled task by default; system service install remains an explicit elevated
compatibility path.

Bootstrap and update are considered successful only after the remote
`descriptor` control request succeeds. The descriptor records the real control
endpoint, transport endpoint, protocol versions, service instance id, remote
user, data directory, and advertised transports. Re-running bootstrap repairs an
existing systemd unit, restarts it, then refreshes the descriptor before local
state is updated.

`ssh_proxy status --json` exposes `peer_store`, `peer_health`, `peer_install`,
and `transport_decision`. `ssh_proxy doctor --json --report --target <host>`
adds a redacted target-specific peer report with install state, service manager,
descriptor state, dependency classification, and matching route decisions.

## OpenSSH Policy

OpenSSH is not a normal fallback. The Rust SSH client and Rust transport engine
own bootstrap and route setup. External OpenSSH may only appear as explicit
emergency compatibility when the daemon reports `requires_external_ssh=true`
with a concrete unsupported capability.

## Troubleshooting

Start with daemon JSON:

```powershell
ssh_proxy daemon status --json
ssh_proxy status --json
ssh_proxy doctor --json --report
```

For a VS Code window:

```powershell
ssh_proxy vscode status --workspace <workspace-id> --json
ssh_proxy events --job <job-id> --json
```

Common cases:

- `502 Bad Gateway`: verify the local proxy URL, scheme, port, and upstream proxy health.
- `daemon_unavailable`: install or start the system daemon interactively.
- `requires_elevation`: run the suggested daemon install/update command with `--elevate`.
- `node_control_token_required`: the running daemon is stale or token-backed; use the interactive repair action to reinstall/migrate the daemon.
- `remote_port_occupied`: keep automatic port picking enabled or select another preferred port.
- `starting`, `ensure_remote_peer`, or `bootstrapping_peer`: inspect events; slow bootstrap is not a reason to switch to OpenSSH.

## Reporting

Prefer `ssh_proxy doctor --json --report` or `Remote Proxy: Diagnose`. Reports
redact daemon tokens, peer tokens, proxy credentials, and SSH identity paths by
default, while keeping phases, blockers, install logs, handoff probes, route
health, peer state, and dependency classification.
