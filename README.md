# ssh_proxy

`ssh_proxy` turns SSH-reachable machines into peer proxy nodes. One binary can
act as CLI, local service daemon, remote service daemon, SSH bootstrap helper,
SOCKS5H/HTTP proxy ingress, and route supervisor.

The data path uses `russh` for SSH fallback instead of shelling out to OpenSSH.
When both nodes can reach each other directly, daemons can use the private
`SPX1` protocol over TLS/TCP, QUIC, or explicitly trusted plain TCP. SSH remains
available for bootstrap, management, ProxyJump-style networks, and emergency
fallback.

## Repository Layout

- `src/`: Rust CLI, daemon, route planning, transports, SSH client, and protocol modules.
- `tests/`: Rust integration and contract tests.
- `scripts/`: release, cross-build, benchmark, and VS Code extension staging helpers.
- `apps/vscode-remote-proxy/`: VS Code UI extension that can use `ssh_proxy` as its route kernel.
- `docs/`: public architecture, operations, repository, release, and license documentation.

More detail lives in [docs/architecture.md](docs/architecture.md),
[docs/operations.md](docs/operations.md), and [docs/repository.md](docs/repository.md).

## Build

Run the normal contributor check from the repository root:

```powershell
pwsh -NoProfile -File scripts/check-all.ps1
```

The script runs Rust formatting, `cargo check`, Rust tests, and the VS Code
extension TypeScript/Node test suite. For CI or local checks that do not need an
embedded Linux helper, it sets `SSH_PROXY_ALLOW_MISSING_SIDECAR=1`.

Release builds embed a Linux musl helper sidecar so a non-Linux client can
bootstrap a Linux remote:

```powershell
pwsh -NoProfile -File scripts/build-release.ps1
```

```bash
scripts/build-release.sh
```

Package the VS Code extension with staged kernel binaries:

```powershell
pwsh -NoProfile -File scripts/package-vscode-extension.ps1
```

See [docs/release.md](docs/release.md) for release and packaging details.

## Quick Start

Install the local user service:

```powershell
ssh_proxy service --scope user install
ssh_proxy service status
```

Bootstrap or refresh a remote peer through SSH:

```powershell
ssh_proxy host <remote-host> --accept-new --persist auto start
ssh_proxy host <remote-host> node-status
```

Create a local listener that uses the remote machine as egress:

```powershell
ssh_proxy route <remote-host> `
  --direction local-uses-remote `
  --port <local-proxy-port>
```

Create a remote listener that uses this machine as egress:

```powershell
ssh_proxy route <remote-host> `
  --direction remote-uses-local `
  --port <remote-proxy-port> `
  --connect-mode reverse-link
```

When both daemons have directly reachable peer transports, use a direct peer
plan instead:

```powershell
ssh_proxy route <remote-host> `
  --direction remote-uses-local `
  --port <remote-proxy-port> `
  --connect-mode direct `
  --local-peer <local-peer-host>:<local-peer-port>
```

If the egress side must chain through an existing proxy, attach it to the route:

```powershell
ssh_proxy route <remote-host> `
  --direction remote-uses-local `
  --port <remote-proxy-port> `
  --connect-mode reverse-link `
  --egress-proxy <proxy-url>
```

Supported upstream schemes are `http://` for CONNECT proxies and
`socks5h://`/`socks5://` for no-auth SOCKS5 proxies.

## Fixed TCP Tunnels

Use `--tcp-target host:port` when the listener should expose one fixed TCP
destination instead of SOCKS5H/HTTP proxy semantics:

```powershell
ssh_proxy route <remote-host> `
  --direction local-uses-remote `
  --port <local-listen-port> `
  --tcp-target <target-host>:<target-port>
```

The listener stays on the side selected by `--direction`; the TCP target is
opened from the egress side.

## Transport Choices

`remote_transport = "auto"` prefers configured direct peer transports and falls
back to SSH when direct paths are unavailable.

- `ssh-native`: SSH `direct-tcpip` data path for simple SSH-only egress.
- `tcp`: SPX framed data path over SSH `direct-tcpip` to a remote daemon transport.
- `tls-tcp`: encrypted direct peer transport and the recommended production direct baseline.
- `quic`: framed SPX over QUIC.
- `quic-native`: peer-native QUIC with one proxied TCP flow per QUIC bidirectional stream.
- `plain-tcp`: explicit lab or trusted-network baseline only.

Use `route --explain` before starting a route when you need to inspect topology,
fallback, selected transport, and runtime settings:

```powershell
ssh_proxy route <remote-host> `
  --direction local-uses-remote `
  --port <local-proxy-port> `
  --explain
```

## VS Code Extension

The extension under `apps/vscode-remote-proxy` can use `ssh_proxy` as its
forwarding kernel instead of launching `ssh -R` directly. The extension stays a
VS Code UI extension because it needs local access to proxy detection, SSH
configuration, the `ssh_proxy` executable, and the local daemon.

The extension's default kernel route is a session-scoped `remote-uses-local`
reverse-link route. It records ownership, keeps a per-user lease, reuses healthy
routes across compatible VS Code windows, and prefers the previous remote port
for reconnects before trying a new port.

## Operations

Common operational topics are documented in [docs/operations.md](docs/operations.md):

- service installation and bootstrap flow;
- peer records, tokens, and certificate material;
- route ownership and restore behavior;
- version and authentication reconciliation;
- route status and troubleshooting.

## License

This repository is distributed under the MIT License. See [LICENSE](LICENSE)
and [docs/license.md](docs/license.md).
