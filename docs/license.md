# License

`ssh_proxy` is distributed under the MIT License. The root `LICENSE` file is the
canonical license for the Rust binary, scripts, documentation, and in-repository
VS Code extension source unless a file explicitly says otherwise.

The VS Code extension also carries its own `apps/vscode-remote-proxy/LICENSE`
file so the extension package is self-contained when published separately.

Generated release binaries and VSIX packages contain this project's code under
MIT. Third-party crates and npm packages remain under their own licenses; use
`Cargo.lock` and `apps/vscode-remote-proxy/package-lock.json` to identify exact
dependency versions for release review.

Do not commit private keys, daemon tokens, peer tokens, proxy credentials, or
machine-specific service files into the repository.
