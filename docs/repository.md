# Repository Guide

This repository is a single source tree for the `ssh_proxy` Rust binary and the
VS Code extension that can use that binary as its forwarding kernel.

## Top-Level Layout

- `Cargo.toml`, `Cargo.lock`: Rust workspace metadata, shared dependencies, and
  release profiles.
- `crates/ssh-proxy/`: default Rust package containing the `ssh_proxy` binary,
  sidecar build script, modules, and integration tests.
- `apps/vscode-remote-proxy/`: VS Code UI extension.
- `scripts/`: build, check, release, benchmark, and extension packaging helpers.
- `docs/`: public architecture, operations, repository, release, and license
  documentation.
- `.github/`: CI and GitHub contribution templates.

Machine-specific notes, local tool configuration, benchmark inputs, and
generated artifacts are ignored by Git and should stay out of public commits.
Benchmark and E2E configuration belongs in untracked local environment files
such as `scripts/bench.local.ps1`; commit only sanitized example templates.

## Rust Workspace Organization

The Rust code is a Cargo workspace with a virtual root manifest. The default
member is `crates/ssh-proxy`, so normal root-level Cargo commands still build
the `ssh_proxy` binary while shared metadata, profiles, and layered crates live
at the workspace root.

Current horizontal crates:

- `crates/ssh-proxy-core/`: shared report, repair-action, command-output, and
  redaction primitives. It must stay free of async runtimes, SSH, CLI, and
  service-manager dependencies.
- `crates/ssh-proxy-protocol/`: protocol DTOs, control envelopes, descriptor
  models, and SPX/QNC1-compatible codec helpers.
- `crates/ssh-proxy-lifecycle/`: lifecycle specs, artifacts, stores, reports,
  fake/local executor contracts, and provider workflow models.
- `crates/ssh-proxy-config/`: app paths and atomic/private file helpers.
- `crates/ssh-proxy-control/`: daemon control socket endpoints, JSON-line
  request/response I/O limits, Unix sockets, and Windows named-pipe ACL setup.
- `crates/ssh-proxy-ssh/`: Rust-native SSH target resolution, OpenSSH config
  parsing, agent/private-key authentication, jump chains, exec/upload, and
  direct-tcpip streams.
- `crates/ssh-proxy-transport/`: peer transport contracts, TLS/QUIC helpers,
  and QUIC stream adapters.
- `crates/ssh-proxy-route/`: route runtime decision reports, preflight
  metadata, route task status records, and route status JSON contracts.
- `crates/ssh-proxy-deploy/`: remote install result DTOs and command-neutral
  remote setup artifact intents.
- `crates/ssh-proxy-service/`: local service-management contracts and provider
  report DTOs.
- `crates/ssh-proxy-daemon/`: command-neutral daemon job, session, peer,
  update, state, and request-view DTOs.
- `crates/ssh-proxy-cli/`: Clap command and argument contracts. The binary
  crate converts these intents into daemon, lifecycle, deploy, or route calls.

`crates/ssh-proxy/` is now the vertical application crate. It keeps the
`ssh_proxy` binary, mimalloc allocator, logging/runtime bootstrap, build script,
and thin shim modules for compatibility with existing internal paths. Remaining
large vertical subsystems are still split by semantic module:

- `deploy/`: app adapters for remote bootstrap entrypoints, descriptor/token
  helpers, compatibility helpers, and transport opening.
- `node_daemon/`: daemon runtime orchestration, control protocol adapters,
  route supervision, peer management, and peer transport listeners.
- `node_daemon/proxy_session/`: reusable state-machine helpers for session
  reuse, route readiness, handoff, and setup sequencing.
- `node_daemon/remote_setup/`: payload rendering and SSH execution adapters for
  deploy-owned VS Code settings, server-env setup, and status-file intents.
- `quic_native/`: QUIC-native control and per-flow stream runtime.
- `service/`: local service planning and platform execution.
- `socks/`: SOCKS5H, HTTP proxy parsing, and relay helpers.

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
