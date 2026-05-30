# Testing

Use layered checks during development. The goal is to keep the common edit loop
fast while still preserving full release confidence.

## Fast Edit Loop

Prefer the `rtk` native wrappers for normal targeted checks. PowerShell and CMD
scripts are compatibility or batch gates; use them when you need their cleanup
behavior, grouped options, or environment setup. `sccache` is supported in the
local toolchain, so repeated `rtk cargo ...` checks should stay fast after the
first build.

The common per-topic gate is:

```powershell
rtk cargo fmt --all -- --check
rtk cargo test -p ssh_proxy --test build_contract
rtk cargo check --workspace --tests
```

Tests follow ownership boundaries. Pure `Spec`, `Intent`, `Plan`, `Policy`,
`Report`, and `Decision` behavior belongs in the crate that owns that type:
route policy in `ssh-proxy-route`, proxy session DTOs in `ssh-proxy-daemon`,
service health and peer compatibility in `ssh-proxy-service`, and provider or
remote setup rendering in `ssh-proxy-deploy`, `ssh-proxy-lifecycle`, or
`ssh-proxy-platform`. The app crate tests should cover CLI/config adapters,
legacy daemon JSON-line compatibility, real Tokio listener/probe behavior, and
small smoke tests that need the `ssh_proxy` binary.

Run the default fast gate after normal daemon, CLI, or extension edits:

```powershell
pwsh -NoProfile -File scripts/check-fast.ps1 -SkipVscode
```

This runs:

- `cargo check --workspace --tests`
- protocol/control/data/report DTO contract tests
- peer lifecycle/config/provider contract tests
- deploy/remote install lifecycle tests
- handoff unit tests
- one daemon route lifecycle smoke test

Add `-Contracts` when CLI shape, release profile, or public command surfaces
change. Add `-Transport` only when transport routing or data-plane behavior
changes. The transport smoke is intentionally single-threaded because it starts
long-lived local daemon/proxy processes and binds ephemeral ports.

The default gate intentionally tests subsystem bodies first. If it fails, rerun
the failing targeted test with logs or run `-Full` for broader integration
context. Avoid jumping straight to the release gate during normal edit loops.

Add targeted Rust tests instead of the full suite when only one subsystem moved:

- protocol envelopes, command aliases, descriptor DTOs, SPX/QNC1 framing, and
  shared report DTOs: `cargo test -p ssh-proxy-protocol`;
- peer lifecycle schema/provider/config/connection metadata:
  `cargo test -p ssh-proxy-lifecycle` for shared lifecycle contracts, and
  `cargo test -p ssh_proxy --bin ssh_proxy peer_lifecycle` for app adapters;
- CLI argument contracts and hidden/public command shape:
  `cargo test -p ssh-proxy-cli`;
- path and atomic file store helpers: `cargo test -p ssh-proxy-config`;
- control socket endpoints, JSON-line request limits, Unix socket and named
  pipe helpers: `cargo test -p ssh-proxy-control`;
- daemon command-neutral job/session/update/control DTOs:
  `cargo test -p ssh-proxy-daemon`;
- platform command/script plan classification:
  `cargo test -p ssh-proxy-platform`;
- Rust-native SSH config parsing, auth helpers, jump chains, exec/upload API:
  `cargo test -p ssh-proxy-ssh`;
- data-plane frame compatibility: `cargo test -p ssh-proxy-protocol`;
- peer transport handshake and TLS/QUIC config contracts:
  `cargo test -p ssh-proxy-transport`;
- QUIC-native control framing: `cargo test -p ssh_proxy --bin ssh_proxy quic_native`;
- remote install lifecycle execution: `cargo test -p ssh_proxy --bin ssh_proxy deploy`;
- remote setup artifact writes: `cargo test -p ssh_proxy --bin ssh_proxy remote_setup`;
- self-update plan/execution adapters:
  `cargo test -p ssh_proxy --bin ssh_proxy update`;
- proxy session spec/state-machine boundaries:
  `cargo test -p ssh_proxy --bin ssh_proxy proxy_session`;
- remote peer file command rendering: `cargo test -p ssh_proxy --bin ssh_proxy remote_config_write`;
- local service lifecycle reporting: `cargo test -p ssh_proxy --bin ssh_proxy service`;
- service health and peer compatibility DTOs:
  `rtk cargo test -p ssh-proxy-service`, `rtk cargo test -p ssh-proxy-protocol`,
  `rtk cargo test -p ssh_proxy --bin ssh_proxy service`, and
  `rtk cargo test -p ssh_proxy --bin ssh_proxy diagnostics`;
- route transport decisions and daemon route metadata: `cargo test -p ssh_proxy --bin ssh_proxy routes`;
- pure route conflict, pool, preflight, fallback, remote-use, and route report
  policy: `rtk cargo test -p ssh-proxy-route`;
- daemon control JSON-line contracts:
  `rtk cargo test -p ssh_proxy --test node_daemon_control`;
- daemon route persistence/recovery smoke:
  `rtk cargo test -p ssh_proxy --test node_daemon_routes -- --test-threads=1`;
- transport runtime smoke:
  `rtk cargo test -p ssh_proxy --test transport_smoke -- --test-threads=1`;
- protocol/transport matrix harness shape:
  `rtk cargo test -p ssh_proxy --test transport_matrix`;
- SPX runtime report and proxy tunnel adapter changes:
  `cargo test -p ssh_proxy --bin ssh_proxy controller socks`;
- repair/report schema: `cargo test -p ssh_proxy --bin ssh_proxy repair diagnostics`;
- workspace dependency, runtime boundary, and external execution contracts:
  `rtk cargo test -p ssh_proxy --test build_contract`;
- extension command shape: `rtk npm --prefix apps/vscode-remote-proxy test`.

Prefer the pure lifecycle/provider tests while editing service managers. They use
fake executors and command rendering contracts, so they do not install services,
open long-lived routes, or keep `target/debug/ssh_proxy.exe` locked on Windows.

Run the VS Code tests separately when extension code changes:

```powershell
rtk npm --prefix apps/vscode-remote-proxy test
```

## Remote E2E Gate

Real SSH tests are opt-in and never part of the normal per-commit gate. Keep
private host aliases and jump topology in local environment files only. The
tracked example is `scripts/remote-e2e.local.example.ps1`; copy it to ignored
`scripts/remote-e2e.local.ps1` or set the variables in your shell.

Remote levels:

- `probe`: verifies OpenSSH reachability and `ssh_proxy host exec` russh parity
  for declared target aliases. It does not write remote files.
- `smoke`: uploads the release musl sidecar, starts a temporary remote daemon
  under `/tmp/ssh_proxy-e2e-*`, checks daemon status/routes, and cleans up.
- `full`: extends smoke with own-binary `remote admin` checksum/status checks
  and fallback classification assertions.

Required opt-in and common knobs:

```powershell
rtk cargo zigbuild -p ssh_proxy --target x86_64-unknown-linux-musl --release
$env:SSH_PROXY_REMOTE_E2E = "1"
$env:SSH_PROXY_REMOTE_LEVEL = "probe" # probe, smoke, or full
$env:SSH_PROXY_REMOTE_TARGETS = "proxyjump-alias,direct-alias"
$env:SSH_PROXY_REMOTE_JUMP_TARGET = "proxyjump-alias"
$env:SSH_PROXY_REMOTE_DIRECT_TARGET = "direct-alias"
$env:SSH_PROXY_REMOTE_ACCEPT_NEW = "0"
$env:SSH_PROXY_REMOTE_KEEP = "0"
```

Run layers explicitly:

```powershell
rtk cargo test -p ssh_proxy --test remote_e2e -- --ignored remote_probe --test-threads=1
rtk cargo test -p ssh_proxy --test remote_e2e -- --ignored remote_smoke --test-threads=1
rtk cargo test -p ssh_proxy --test remote_e2e -- --ignored remote_full --test-threads=1
```

Remote runs report only target alias, topology class, cleanup status, and
failure classification. Each target gets a stamp, token, remote directory,
daemon pidfile, control port, and transport port. Failed runs still clean up
unless `SSH_PROXY_REMOTE_KEEP=1` is set for investigation.

## Protocol/Transport Matrix Gate

Use the matrix gate when protocol speed, stability, or connection selection
policy changes. It is ignored and environment-gated like remote E2E, but it
keeps strategy, data-plane correctness, and trend measurements in one artifact.
Run it serially; do not start multiple ignored matrix or remote E2E cargo
commands in parallel on Windows because the linker can contend for the same test
binary.

Layering:

- `matrix_probe`: checks local release binary, Linux sidecar, `ssh`/`scp`/`curl`,
  target topology, OpenSSH reachability, russh `host exec`, and remote `/tmp`
  permissions.
- `matrix_smoke`: starts an isolated remote daemon and verifies fixed-target
  data-plane status through `ssh-native`, SPX over SSH, and direct
  plain/TLS/QUIC/QUIC-native when the topology is direct.
- `matrix_perf_smoke`: repeats the same correctness cases with low concurrency
  and records bytes, batch-wall-clock duration, MiB/s, first-byte latency,
  `measurement_scope`, `sample_count`, `request_count`, and `concurrency` as
  report-first trend data. The first scope is `control-status-through-proxy`,
  so its MiB/s is a relative transport trend, not a large-object throughput
  benchmark.
- `matrix_stability`: runs longer repeated status probes and records lost
  requests, reconnect count, and `run_window_ms`; set a short duration for
  development.

Common configuration:

```powershell
rtk cargo build -p ssh_proxy --release
rtk cargo zigbuild -p ssh_proxy --target x86_64-unknown-linux-musl --release
$env:SSH_PROXY_MATRIX = "1"
$env:SSH_PROXY_MATRIX_LEVEL = "smoke" # probe, smoke, perf-smoke, or stability
$env:SSH_PROXY_MATRIX_TARGETS = "proxyjump-alias,direct-alias"
$env:SSH_PROXY_MATRIX_JUMP_TARGET = "proxyjump-alias"
$env:SSH_PROXY_MATRIX_DIRECT_TARGET = "direct-alias"
$env:SSH_PROXY_MATRIX_ACCEPT_NEW = "0"
$env:SSH_PROXY_MATRIX_KEEP = "0"
```

Run layers explicitly:

```powershell
rtk cargo test -p ssh_proxy --test transport_matrix -- --ignored matrix_probe --test-threads=1
rtk cargo test -p ssh_proxy --test transport_matrix -- --ignored matrix_smoke --test-threads=1
rtk cargo test -p ssh_proxy --test transport_matrix -- --ignored matrix_perf_smoke --test-threads=1
$env:SSH_PROXY_MATRIX_DURATION_SECS = "300"
rtk cargo test -p ssh_proxy --test transport_matrix -- --ignored matrix_stability --test-threads=1
```

The matrix writes `transport-matrix.json` and `transport-matrix.csv` under a
temporary artifact directory, or `SSH_PROXY_MATRIX_ARTIFACT_DIR` when set.
Correctness, cleanup, and classified failures are hard failures. Throughput,
latency, and reconnect observations are report-first until several lab runs
establish stable thresholds. For concurrent perf-smoke rows, `duration_ms` is
the sum of per-sample batch wall-clock times, while `run_window_ms` is the total
measurement phase for that case. The legacy PowerShell benchmark scripts remain
compatibility/lab wrappers; prefer the Rust matrix gate for release evidence
because it uses `rcgen` in the test harness instead of an external `openssl.exe`.

## Full Local Gate

Run full mode before handoff, before packaging, or when a smoke test fails and
the failure needs broader context:

```powershell
pwsh -NoProfile -File scripts/check-fast.ps1 -Full
```

`-Full` uses `cargo nextest run --workspace --tests` when available. Without nextest, it
falls back to single-threaded `cargo test --workspace --tests` to reduce port races and
long-lived child-process overlap in integration tests.

Use `scripts/check-all.ps1` when you also want formatting:

```powershell
pwsh -NoProfile -File scripts/check-all.ps1
```

## Child Process Cleanup

Some integration tests start `ssh_proxy` daemon/proxy child processes. They are
wrapped in a test `ChildGuard`, so panic paths kill and wait on children before
the test binary exits. This prevents Windows from keeping
`target\debug\ssh_proxy.exe` locked after a failed test.

The check scripts also clean stale test binaries before and after Rust checks.
They only target the workspace debug binary:

```text
target\debug\ssh_proxy.exe
```

They do not stop the production system daemon installed under ProgramData. Pass
`-NoProcessCleanup` (`--no-process-cleanup` on shell scripts) only when you are
intentionally running the workspace debug binary outside the test harness.

## Release Gate

Before publishing, keep the full release gate explicit:

```powershell
rtk cargo test --workspace --tests
rtk cargo build -p ssh_proxy --release
rtk cargo zigbuild -p ssh_proxy --target x86_64-unknown-linux-musl --release
rtk npm --prefix apps/vscode-remote-proxy test
rtk npm --prefix apps/vscode-remote-proxy run package:with-kernel
```
