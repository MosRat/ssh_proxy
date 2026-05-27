# Release and Packaging

`ssh_proxy` release artifacts have two parts:

- the Rust binary, which embeds a Linux musl helper sidecar for remote bootstrap;
- the VS Code extension package, which can optionally bundle platform binaries.

## Prerequisites

- Rust stable toolchain.
- `cargo-zigbuild` and Zig for Linux musl cross builds.
- PowerShell 7 for the Windows-first release scripts.
- Node.js 20 or newer for the VS Code extension.
- `npm ci` run inside `apps/vscode-remote-proxy` before packaging the extension.

## Check

```powershell
pwsh -NoProfile -File scripts/check-all.ps1
```

The check script sets `SSH_PROXY_ALLOW_MISSING_SIDECAR=1` for non-release Rust
checks, then runs:

- `cargo fmt -- --check`
- `cargo check`
- `cargo test --tests`
- `npm --prefix apps/vscode-remote-proxy test`

For a local production gate before packaging or publishing, run the explicit
commands as well:

```powershell
cargo test --tests
cargo build --release
cargo zigbuild --target x86_64-unknown-linux-musl --release
npm --prefix apps/vscode-remote-proxy test
npm --prefix apps/vscode-remote-proxy run package:with-kernel
```

## Rust Release Binary

```powershell
pwsh -NoProfile -File scripts/build-release.ps1
```

The script builds:

1. `target/x86_64-unknown-linux-musl/release/ssh_proxy`
2. the host release binary with `SSH_PROXY_LINUX_MUSL_BIN` pointing at that
   musl helper

Use the shell variant on Unix-like systems:

```sh
scripts/build-release.sh
```

## VS Code Extension Package

```powershell
pwsh -NoProfile -File scripts/package-vscode-extension.ps1
```

By default this builds the Rust release binaries, stages them into
`apps/vscode-remote-proxy/assets/bin`, then runs `npm run package` in the
extension directory.

Use `-SkipBuild` when the release binaries are already present:

```powershell
pwsh -NoProfile -File scripts/package-vscode-extension.ps1 -SkipBuild
```

The staged binaries are ignored by Git and should not be committed.

## GitHub Actions Release

The release workflow lives in `.github/workflows/release.yml`.

It runs on:

- tags matching `v*`;
- manual `workflow_dispatch`.

The workflow builds and uploads:

- `ssh_proxy-x86_64-unknown-linux-musl.tar.gz`
- `ssh_proxy-x86_64-unknown-linux-gnu.tar.gz`
- `ssh_proxy-aarch64-unknown-linux-gnu.tar.gz`
- `ssh_proxy-x86_64-pc-windows-msvc.zip`
- `ssh_proxy-aarch64-pc-windows-msvc.zip`
- `ssh_proxy-x86_64-apple-darwin.tar.gz`
- `ssh_proxy-aarch64-apple-darwin.tar.gz`
- `vscode-remote-proxy-<version>-with-kernel.vsix`
- matching `.sha256` files

The VSIX bundles:

- `assets/bin/win32-x64/ssh_proxy.exe`
- `assets/bin/linux-x64/ssh_proxy`

The release body and job summary include the artifact list, sizes, and SHA256
lines so release verification does not require opening every job log.

Manual workflow inputs:

- `tag_name`: release tag override; defaults to the current ref name.
- `draft`: create the GitHub Release as a draft.
- `prerelease`: mark it as prerelease.
- `publish_vscode`: publish the generated VSIX to the VS Code Marketplace.

## GitHub Secrets

The VS Code Marketplace publish job expects a repository secret named
`VSCE_PAT`. This is the Visual Studio Marketplace personal access token used by
`vsce publish`.

Set it with GitHub CLI:

```powershell
gh secret set VSCE_PAT --repo MosRat/ssh_proxy
```

The workflow never prints the token. The publish job fails early if the secret
is missing and `publish_vscode` is enabled.

## Cache Strategy

The CI and release workflows cache:

- Cargo registry index/cache and Git checkouts;
- `target/` build outputs keyed by OS, architecture, target triple, Rust
  toolchain cache key, and `Cargo.lock`;
- npm cache keyed by `apps/vscode-remote-proxy/package-lock.json`.

Release jobs keep artifacts for 14 days before the GitHub Release is created.
The release itself stores the final archives and checksums.

## Versioning Checklist

- Update `Cargo.toml` package version.
- Update `apps/vscode-remote-proxy/package.json` version.
- Rebuild and retest with `scripts/check-all.ps1`.
- Build the release binary with `scripts/build-release.ps1`.
- Package the extension with `scripts/package-vscode-extension.ps1`.
- Tag the release after the artifacts are verified.
