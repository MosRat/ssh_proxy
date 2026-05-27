# ssh_proxy

`ssh_proxy` is a Rust-native SSH bootstrap and proxy routing tool. One binary can run as a CLI, a local daemon, a remote helper, a route supervisor, and a SOCKS5H/HTTP proxy ingress.

It is built for Remote SSH workflows where a machine behind SSH needs reliable access through another machine's proxy, while still leaving room for direct peer transports when both sides are reachable.

## What It Does

- Creates local or remote proxy listeners with SSH fallback.
- Uses `russh` for native SSH bootstrap and management.
- Supports SOCKS5H and HTTP proxy ingress.
- Supports fixed TCP tunnels with `--tcp-target`.
- Can chain egress through an existing HTTP CONNECT or SOCKS5 proxy.
- Runs persistent local/remote daemons, or session-scoped routes when service installation is unavailable.
- Provides JSON status, route explain output, and route health data for automation.

## Quick Start

Discover, reuse, or repair the persistent local service:

```powershell
ssh_proxy service --json ensure
ssh_proxy service --json status
```

`service ensure` probes existing user/system service scopes first, reuses a healthy control endpoint when one exists, and only repairs or installs when no usable service is available. `service install` remains available for direct installs and accepts `--elevate` when an explicit elevated system install is intended.

Bootstrap or refresh a remote peer through SSH:

```powershell
ssh_proxy host <remote-host> --accept-new --persist auto start
ssh_proxy host <remote-host> node-status
```

Expose a local listener that uses the remote machine as egress:

```powershell
ssh_proxy route <remote-host> `
  --direction local-uses-remote `
  --port <local-proxy-port>
```

Expose a remote listener that uses this machine as egress:

```powershell
ssh_proxy route <remote-host> `
  --direction remote-uses-local `
  --connect-mode reverse-link `
  --port <remote-proxy-port> `
  --egress-proxy http://127.0.0.1:10808/
```

Inspect the selected plan before starting a route:

```powershell
ssh_proxy route <remote-host> `
  --direction remote-uses-local `
  --connect-mode reverse-link `
  --port <remote-proxy-port> `
  --explain
```

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

The extension in `apps/vscode-remote-proxy` automatically exposes a local proxy inside a VS Code Remote SSH window. Its recovery order is:

1. reuse an existing `ssh_proxy` service/control endpoint
2. repair or install a user service when allowed
3. use a session-owned daemon when persistent service setup is unavailable
4. fall back to OpenSSH reverse forwarding only after the kernel path is exhausted

The extension remains a VS Code UI extension because it needs local access to proxy detection, SSH config, the `ssh_proxy` executable, and the local daemon/session process.

See [apps/vscode-remote-proxy/README.md](apps/vscode-remote-proxy/README.md) for usage, settings, troubleshooting, and packaging details.

## Troubleshooting

Use JSON status first:

```powershell
ssh_proxy service --json status
ssh_proxy node control --json routes
```

Common checks:

- If a remote shell returns `502 Bad Gateway`, verify the local upstream proxy URL and make sure the local proxy accepts CONNECT/SOCKS traffic.
- If Windows service installation is denied, use `service --json status` to inspect `selected_control`, `candidates`, and `next_action`. The VS Code extension caches the denied scope for the current window, uses a session daemon, and only then considers OpenSSH.
- If the remote port is occupied, use `remoteProxy.remote.autoPickPort` in the extension or choose another `--port`.
- If a route stays in `accepted`, `bootstrapping_peer`, or `starting`, inspect `node control routes`; route readiness fields are additive diagnostics and the extension waits before applying remote proxy settings.

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

- `src/`: Rust CLI, daemon, route planning, transports, SSH client, proxy ingress, and service code.
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
