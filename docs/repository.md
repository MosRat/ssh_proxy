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
such as `scripts/bench.local.ps1` or `scripts/remote-e2e.local.ps1`; commit
only sanitized example templates.

## Rust Workspace Organization

The Rust code is a Cargo workspace with a virtual root manifest. The default
member is `crates/ssh-proxy`, so normal root-level Cargo commands still build
the `ssh_proxy` binary while shared metadata, profiles, and layered crates live
at the workspace root.

Current horizontal crates:

- `crates/ssh-proxy-core/`: shared report, repair-action, command-output,
  command-neutral intent, model, and redaction primitives. It must stay free of
  async runtimes, SSH, CLI, and service-manager dependencies.
- `crates/ssh-proxy-protocol/`: protocol DTOs, control envelopes, descriptor
  models, and SPX/QNC1-compatible codec helpers.
- `crates/ssh-proxy-lifecycle/`: lifecycle specs, artifacts, stores, reports,
  fake/local executor contracts, and provider workflow models.
- `crates/ssh-proxy-config/`: app paths, atomic/private file helpers, config
  schema, peer records, token metadata, and profile/default planning into core
  intent-shaped values.
- `crates/ssh-proxy-control/`: daemon control socket endpoints, JSON-line
  request/response I/O limits, Unix sockets, and Windows named-pipe ACL setup.
- `crates/ssh-proxy-ssh/`: Rust-native SSH target resolution, OpenSSH config
  parsing, agent/private-key authentication, jump chains, exec/upload,
  direct-tcpip streams, and the opaque `SshStream` wrapper that hides `russh`
  from app runtimes.
- `crates/ssh-proxy-transport/`: peer transport contracts, TLS/QUIC helpers,
  QUIC stream adapters, remote helper opener runtime, peer listener runtime,
  remote helper stream/error models, SPX worker status DTOs, and SOCKS5/HTTP
  proxy parser primitives.
- `crates/ssh-proxy-route/`: route runtime decision reports, route conflict
  policy, route plan rendering, pool sizing policy, preflight probe
  classification, fallback decisions, remote-use decisions, route task status
  records, and route status JSON contracts.
- `crates/ssh-proxy-deploy/`: remote install result DTOs, command-neutral
  remote install plans, remote admin intents, remote setup artifact intents,
  and remote setup script rendering.
- `crates/ssh-proxy-service/`: local service-management contracts, provider
  report DTOs, service health reports, endpoint/binary/route-store summaries,
  peer compatibility reports, service status summaries, candidate summaries,
  selected-control summaries, and fallback recommendations.
- `crates/ssh-proxy-platform/`: local platform command plans, command outcomes,
  script plans, subprocess capture/spawn helpers, and external execution
  classification for service adapters and self-update.
- `crates/ssh-proxy-daemon/`: command-neutral daemon job, session, peer,
  update, state, proxy session spec, request intent/payload DTOs, and daemon
  client fallback reports.
- `crates/ssh-proxy-cli/`: Clap command and argument contracts plus adapters
  into core command-neutral intents. The binary crate converts those intents
  into daemon, lifecycle, deploy, or route calls.

`crates/ssh-proxy/` is now the vertical application crate. It keeps the
`ssh_proxy` binary, mimalloc allocator, logging/runtime bootstrap, build script,
and thin shim modules for compatibility with existing internal paths. Remaining
large vertical subsystems are still split by semantic module:

- `deploy/`: app adapters for remote bootstrap entrypoints, descriptor/token
  helpers, compatibility helpers, and transport opening.
- `node_daemon/`: daemon runtime orchestration, control protocol adapters,
  route supervision, peer management, and peer transport listeners.
- `node_daemon/control_protocol/`: legacy JSON-line response rendering and
  typed payload adapters for command-neutral daemon intents.
- `node_daemon/proxy_session/`: CLI-to-session-spec adapter plus reusable
  state-machine helpers for session reuse, route readiness, handoff, and setup
  sequencing.
- `node_daemon/remote_setup/`: SSH execution adapters for deploy-owned VS Code
  settings, server-env setup, status-file intents, and fallback scripts.
- `node_daemon/management/update.rs`: app-side self-update orchestration using
  daemon update DTOs and platform command/script plans.
- `quic_native/`: QUIC-native control and per-flow stream runtime.
- `service/`: local service planning, status adapters, and platform execution.
- `socks/`: app-side SOCKS5H/HTTP listener dispatch, relay helpers, and
  outbound adapters. Protocol parsing lives in `ssh-proxy-transport`.

Integration tests under `crates/ssh-proxy/tests/` are split by runtime surface:
`node_daemon_control` covers daemon JSON-line/control contracts,
`node_daemon_routes` covers route persistence/recovery smoke,
`transport_smoke` covers local data-plane runtime behavior, and `remote_e2e`
contains ignored real SSH probe/smoke/full gates. Shared test code lives under
`crates/ssh-proxy/tests/support/`; large harnesses should use submodules rather
than accumulating long single-file tests.

Intent/runtime layering rules:

- Lower crates expose `Spec`, `Intent`, `Plan`, `Policy`, `Report`, and
  `Decision` shapes without depending on CLI parsing or app runtime modules.
- App modules may temporarily adapt legacy CLI-shaped structs, but new pure
  planning logic should move to the appropriate workspace crate first.
- Runtime adapters own external mechanisms: SSH exec/upload/direct-tcpip,
  Tokio listeners, local service FFI, and platform command execution.
- External command execution must declare its class at the plan layer:
  provider commands, diagnostic probes, self-update scripts, and emergency
  compatibility paths are not interchangeable.
- Native and own-binary success paths must expose `execution_backend`,
  `fallback_used`, and `external_action`; fallback shell scripts must be
  classified as fallback or compatibility paths.
- Service health and peer compatibility JSON is rendered by
  `ssh-proxy-service`; app modules only provide filesystem, TOML, endpoint, and
  TCP probe inputs.
- Production runtime modules should return structured errors or typed reports
  rather than adding new `unwrap`, `expect`, `panic`, `todo`, or
  `unimplemented` paths. Tests and explicit fake executors are exempt.
- Boundary tests enforce production dependency direction for core/config/route/
  deploy/lifecycle/transport/daemon crates, prevent runtime imports from
  crossing layers, keep `service-manager` out of production dependencies,
  require internal crate dependencies to use workspace declarations, and keep
  production `Command::new` calls inside platform or lifecycle executor crates.

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
