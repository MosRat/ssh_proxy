# Repository Guide

This repository is a single source tree for the `ssh_proxy` Rust binary and the
VS Code extension that can use that binary as its forwarding kernel.

## Top-Level Layout

- `Cargo.toml`, `Cargo.lock`, `build.rs`: Rust package metadata and sidecar
  embedding contract.
- `src/`: Rust CLI, daemon, SSH, route planning, transport, and protocol code.
- `tests/`: Rust integration and contract tests.
- `apps/vscode-remote-proxy/`: VS Code UI extension.
- `scripts/`: build, check, release, benchmark, and extension packaging helpers.
- `docs/`: public architecture, operations, repository, release, and license
  documentation.
- `.github/`: CI and GitHub contribution templates.

Machine-specific notes, local tool configuration, benchmark inputs, and
generated artifacts are ignored by Git and should stay out of public commits.
Benchmark and E2E configuration belongs in untracked local environment files
such as `scripts/bench.local.ps1`; commit only sanitized example templates.

## Rust Module Organization

The Rust crate intentionally stays at the repository root so normal Cargo
commands work without a workspace wrapper. Large subsystems are split into
semantic modules:

- `src/cli.rs`: command and argument contracts.
- `src/ssh_client.rs` and `src/ssh_auth.rs`: russh session, ProxyJump, host key,
  and authentication handling.
- `src/deploy/`: remote command builders, helper upload, and transport opening.
- `src/node_daemon/`: node service control protocol, route supervision, peer
  management, and peer transport listeners.
- `src/quic_native/`: QUIC-native control and per-flow stream runtime.
- `src/service/`: local service planning and platform execution.
- `src/socks/`: SOCKS5H, HTTP proxy parsing, and relay helpers.

See `docs/architecture.md` for the deeper runtime model.

## VS Code Extension Organization

The extension is tracked under `apps/vscode-remote-proxy` rather than a symlink
to another checkout. It remains a UI extension because it needs access to the
local proxy, local SSH config, local `ssh_proxy` binary, and local daemon.

Generated extension files are ignored:

- `node_modules/`
- `out/`
- `*.vsix`
- staged binaries under `assets/bin/*/ssh_proxy*`

## Public Documentation Boundary

Public docs should describe product behavior, operation, architecture, and the
release process. Do not commit personal notes, chat transcripts, ad hoc plans,
raw benchmark outputs, private hostnames, concrete private IP/port assignments,
local filesystem paths, credentials, or secrets.
