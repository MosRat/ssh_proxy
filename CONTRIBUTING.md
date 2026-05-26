# Contributing

Thanks for improving `ssh_proxy`. The repository contains the Rust daemon/CLI and
the VS Code extension that can use it as a local route kernel.

## Repository Shape

- Rust sources live under `src/`.
- Rust integration and contract tests live under `tests/`.
- VS Code extension sources live under `apps/vscode-remote-proxy/`.
- Build, release, benchmark, and packaging helpers live under `scripts/`.
- Public documentation lives under `docs/`.

Keep local planning notes, transcripts, and machine-specific files out of Git.

## Development Checks

From the repository root:

```powershell
pwsh -NoProfile -File scripts/check-all.ps1
```

For targeted work:

```powershell
cargo fmt -- --check
cargo check
cargo test --tests
npm --prefix apps/vscode-remote-proxy test
```

`cargo check` and tests may set `SSH_PROXY_ALLOW_MISSING_SIDECAR=1` when they do
not need to embed a Linux helper. Release builds must use the real sidecar flow.

## Build and Package

```powershell
pwsh -NoProfile -File scripts/build-release.ps1
pwsh -NoProfile -File scripts/package-vscode-extension.ps1
```

The release build first builds the Linux musl helper with `cargo zigbuild`, then
embeds that helper into the host binary. The VS Code package script stages the
release binaries into `apps/vscode-remote-proxy/assets/bin` before invoking
`vsce package`.

## Dependency Policy

- Prefer Rust-native crates and static linking.
- Use `cargo add` for new Rust dependencies.
- Avoid direct C FFI and C build-system dependencies unless there is a clear,
  documented reason.
- Keep `mimalloc` as the global allocator unless a measured regression proves a
  better option.

## Commit Messages

Use Conventional Commits:

```text
<type>(<scope>): <subject>
```

Allowed types are `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`,
`build`, `ci`, `chore`, `revert`.
