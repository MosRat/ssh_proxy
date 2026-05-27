# ssh_proxy

`ssh_proxy` is a Rust-native SSH bootstrap and proxy routing daemon. One binary can run the local Docker-like daemon, act as a thin CLI client, install/update remote peers, and expose SOCKS5H/HTTP proxy routes.

It is built for Remote SSH workflows where a machine behind SSH needs reliable access through another machine's proxy, while still leaving room for direct peer transports when both sides are reachable.

## What It Does

- Creates local or remote proxy listeners through the local daemon.
- Uses `russh` for native SSH bootstrap and management.
- Supports SOCKS5H and HTTP proxy ingress.
- Supports fixed TCP tunnels with `--tcp-target`.
- Can chain egress through an existing HTTP CONNECT or SOCKS5 proxy.
- Uses a single local daemon as the authoritative service, route, peer, job, and health control plane.
- Provides JSON daemon status, job progress, route health, and VS Code-focused integration commands.

## Quick Start

Install or inspect the local daemon:

```powershell
ssh_proxy daemon --json status
ssh_proxy daemon --scope system install --elevate
```

The daemon is the normal production path. CLI commands submit intent to the private named pipe or Unix socket; they do not directly own long-running routes. Non-interactive commands report `requires_elevation` instead of opening a UAC or sudo prompt.

Expose a remote listener that uses this machine as egress through the daemon:

```powershell
ssh_proxy up `
  --target <remote-host> `
  --local-proxy http://127.0.0.1:10808/ `
  --remote-port <remote-proxy-port> `
  --json
```

Inspect the daemon-owned job and route state:

```powershell
ssh_proxy status --json
ssh_proxy events --job <job-id> --json
ssh_proxy doctor --json
```

## Daemon Job Workflow

`ssh_proxy up` and `ssh_proxy vscode up` both submit an `ensure_proxy_session` job to the daemon. The command returns quickly with a job id, route id, and intended remote URL. The daemon then drives the session through these phases:

```text
resolve_target -> ensure_local_proxy -> ensure_peer -> plan_route
  -> start_route -> wait_route_ready -> verify_remote_port
  -> apply_remote_settings -> healthy
```

`ssh_proxy status --workspace <id> --json` reports the current `ProxySessionStatus`. `ssh_proxy events --job <job-id> --json` reports structured job events. If the daemon restarts while a job is unfinished, it restores the latest snapshot and reconciles the proxy session instead of starting from an unknown state.

The daemon is the only production status source for CLI and VS Code paths. Older `route`, `node`, `service`, and `host` commands still exist as hidden compatibility/internal tools, but production workflows should use the daemon commands above.

## Core Modes

`local-uses-remote` creates a local listener and opens target connections from the remote side.

`remote-uses-local` creates a remote listener and opens target connections from the local side. This is the default shape used by the VS Code extension when a remote shell needs the local desktop proxy.

`--connect-mode reverse-link` keeps the route reachable through SSH even when the local machine is not directly reachable from the remote host.

`--connect-mode direct` is for environments where both daemon peers can reach each other directly. Use TLS/TCP or QUIC for production direct links; plain TCP is intended only for explicitly trusted lab networks.

`--tcp-target host:port` turns a proxy listener into a fixed TCP tunnel.

Supported upstream egress proxy schemes:

- `http://` for HTTP CONNECT proxies
- `socks5h://` and `socks5://` for no-auth SOCKS5 proxies

## VS Code Extension

The extension in `apps/vscode-remote-proxy` automatically exposes a local proxy inside a VS Code Remote SSH window. Its normal path is a thin daemon client:

```powershell
ssh_proxy vscode up --target <ssh-host> --workspace <id> --local-proxy <url> --json
```

The daemon owns route startup, readiness, remote setup, job state, and health. The extension no longer exposes OpenSSH or session-daemon fallback as normal settings; external OpenSSH is only an emergency compatibility path reported by daemon diagnostics when Rust SSH cannot support the target.

See [apps/vscode-remote-proxy/README.md](apps/vscode-remote-proxy/README.md) for usage, settings, troubleshooting, and packaging details.

## Troubleshooting

Use JSON status first:

```powershell
ssh_proxy status --json
ssh_proxy events --job <job-id> --json
ssh_proxy doctor --json
```

Common checks:

- If a remote shell returns `502 Bad Gateway`, verify the local upstream proxy URL and make sure the local proxy accepts CONNECT/SOCKS traffic.
- If Windows daemon installation is denied, use `ssh_proxy daemon --json status` and `ssh_proxy doctor --json`. Auto-start does not pop UAC; interactive commands can install or update the daemon explicitly.
- If the remote port is occupied, keep `remoteProxy.remote.autoPickPort` enabled in the extension or choose another `--remote-port`.
- If a route stays in `accepted`, `bootstrapping_peer`, or `starting`, inspect `ssh_proxy status --workspace <id> --json` and `ssh_proxy events --job <job-id> --json`; readiness is represented as daemon job progress.

More operational detail lives in [docs/operations.md](docs/operations.md).

## Build, Test, Release

Run the normal contributor check:

```powershell
pwsh -NoProfile -File scripts/check-all.ps1
```

Run Rust tests directly:

```powershell
cargo test --tests
```

Build an optimized release binary:

```powershell
cargo build --release
```

Build release artifacts with the Linux helper sidecar:

```powershell
pwsh -NoProfile -File scripts/build-release.ps1
```

Package the VS Code extension with staged kernel binaries:

```powershell
pwsh -NoProfile -File scripts/package-vscode-extension.ps1
```

For musl cross builds, use `cargo zigbuild` as documented in the release flow.

## Repository Layout

- `src/`: Rust CLI, daemon, route planning, transports, SSH client, proxy ingress, and daemon install/update code.
- `tests/`: Rust integration and build-contract tests.
- `apps/vscode-remote-proxy/`: VS Code Remote SSH proxy extension.
- `scripts/`: build, package, benchmark, and binary staging helpers.
- `docs/`: architecture, operations, release, repository, and license docs.

Start with [docs/architecture.md](docs/architecture.md), [docs/operations.md](docs/operations.md), and [docs/repository.md](docs/repository.md) when changing internals.

## Engineering Contract

- Use `cargo add` for dependency changes.
- Keep mimalloc as the global allocator.
- Prefer static linking and Rust-native implementations.
- Avoid direct C FFI unless a plan documents why no Rust-native option works.
- Keep tests under `tests/` for integration and contract coverage.
- Use Conventional Commits for commits.

## License

This repository is distributed under the MIT License. See [LICENSE](LICENSE) and [docs/license.md](docs/license.md).
