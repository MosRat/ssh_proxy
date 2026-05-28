# Testing

Use layered checks during development. The goal is to keep the common edit loop
fast while still preserving full release confidence.

## Fast Edit Loop

Run the default fast gate after normal daemon, CLI, or extension edits:

```powershell
pwsh -NoProfile -File scripts/check-fast.ps1 -SkipVscode
```

This runs:

- `cargo check --tests`
- handoff unit tests
- one daemon route lifecycle smoke test

Add `-Contracts` when CLI shape, release profile, or public command surfaces
change. Add `-Transport` only when transport routing or data-plane behavior
changes. The transport smoke is intentionally single-threaded because it starts
long-lived local daemon/proxy processes and binds ephemeral ports.

Run the VS Code tests separately when extension code changes:

```powershell
npm --prefix apps/vscode-remote-proxy test
```

## Full Local Gate

Run full mode before handoff, before packaging, or when a smoke test fails and
the failure needs broader context:

```powershell
pwsh -NoProfile -File scripts/check-fast.ps1 -Full
```

`-Full` uses `cargo nextest run --tests` when available. Without nextest, it
falls back to single-threaded `cargo test --tests` to reduce port races and
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
cargo test --tests
cargo build --release
cargo zigbuild --target x86_64-unknown-linux-musl --release
npm --prefix apps/vscode-remote-proxy test
npm --prefix apps/vscode-remote-proxy run package:with-kernel
```
