# Bundled ssh_proxy Kernel Binaries

This directory is populated by:

```powershell
pwsh -NoProfile -File ../../scripts/stage-vscode-extension-binaries.ps1
```

Expected staged files:

- `win32-x64/ssh_proxy.exe`
- `linux-x64/ssh_proxy`

The extension prefers a non-default explicit `remoteProxy.sshProxy.executable`,
then a matching bundled binary, then `ssh_proxy` from `PATH`. Setting the
executable to an empty string forces PATH-only discovery.
