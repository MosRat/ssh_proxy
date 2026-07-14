# Remote Proxy Auto Forward

VS Code extension that reverse-forwards a local proxy into a remote VS Code window and applies proxy defaults for Remote SSH terminals, VS Code HTTP clients, remote extensions, and Git.

The first supported target is Remote SSH. WSL and Dev Containers are detected but intentionally left as extension points because they need different forwarding strategies.

OpenSSH is the default backend for the `0.0.x` release line. Set
`remoteProxy.backend` to `ssh_proxy` explicitly to use the binary backend, or
to `auto` to prefer `ssh_proxy` and fall back to OpenSSH when it is unavailable.
Remote setup also defaults to OpenSSH; set `remoteProxy.sshProxy.remoteSetup`
to `auto` or `ssh_proxy` only when binary-backed remote commands are desired.

## What it does

- Detects a local proxy from `remoteProxy.localProxy.url`, `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY`, or the configured local proxy port list.
- Starts an SSH reverse tunnel:

  ```text
  remote 127.0.0.1:<remote-proxy-port> -> local 127.0.0.1:<detected-proxy-port>
  ```

  If the preferred remote port is already in use, nearby ports are tried automatically and the selected port is written to the remote proxy settings.

- Applies remote VS Code settings:
  - `http.proxy`
  - `http.proxySupport`
  - `terminal.integrated.env.*`
- Optionally patches the remote VS Code server machine settings file.
- Optionally writes a managed block to `~/.vscode-server/server-env-setup` so remote extension hosts inherit proxy environment variables after reconnect.
- Optionally force-overwrites remote global Git proxy config and each current remote workspace Git repo config.
- Adds a status bar menu with start, stop, restart, diagnostics, local proxy selection, SSH host override controls, output, and settings.
- Reuses one local SSH reverse tunnel across multiple VS Code windows connected to the same SSH user and server. The owning window writes a local heartbeat lease; other windows reuse the remote proxy port and take over automatically if the lease or tunnel goes stale.

## Commands

- `Remote Proxy: Start`
- `Remote Proxy: Stop`
- `Remote Proxy: Restart`
- `Remote Proxy: Apply Remote Settings`
- `Remote Proxy: Clean Remote Settings`
- `Remote Proxy: Pick Local Proxy`
- `Remote Proxy: Pick SSH Host`
- `Remote Proxy: Clear SSH Host Override`
- `Remote Proxy: Open Menu`
- `Remote Proxy: Diagnose`
- `Remote Proxy: Show Output`
- `Remote Proxy: Open Settings`
- `Remote Proxy: Show Status`

The status bar item opens the same menu. Its tooltip shows the active SSH host, local proxy, remote proxy URL, and the latest error.

## Multi-window reuse

`remoteProxy.singleton.reuseEnabled` is enabled by default. Windows connected to the same SSH user and resolved server share one reverse tunnel instead of racing for the same remote port. If the owner window closes, another window detects the stale or unreachable lease and starts a replacement tunnel.

Lease files are stored in a per-local-user namespace under the system temp directory, so multiple local OS users do not share ownership records. A per-target start lock serializes route creation when several VS Code windows open the same host at once; after the first window creates and verifies the route, the other windows re-read the lease and reuse it.

Remote port selection is sticky inside a VS Code connection window. Reconnects try the current in-memory port first, then the last successful port, then the local lease port, then the remote `~/.vscode-server/remote-proxy-status.json` port written by this extension, and only then the configured port range. When a preferred port is already listening, the extension checks whether it matches the previous Remote Proxy status and verifies it before reusing the leftover listener.

Health checks are controlled by `remoteProxy.forward.healthCheckEnabled`. When enabled, the extension periodically verifies the remote forwarded port and restarts or takes over if it stops responding.

Transient network blips are tolerated through `remoteProxy.forward.healthCheckFailureThreshold`, which defaults to two consecutive failures before restarting. Deterministic remote port conflicts, including `EADDRINUSE` style errors from either OpenSSH or `ssh_proxy`, are treated as retryable when `remoteProxy.remote.autoPickPort` is enabled, so the extension moves through the configured remote port range instead of failing on the first occupied port.

Repeated restart attempts use exponential backoff up to `remoteProxy.forward.restartBackoffMaxSeconds`, which helps during VPN changes, laptop sleep/resume, or temporary SSH outages.

SSH keepalive is configurable through `remoteProxy.ssh.serverAliveInterval`, `remoteProxy.ssh.serverAliveCountMax`, and `remoteProxy.ssh.tcpKeepAlive`. The child tunnel also uses `ExitOnForwardFailure=yes` so failed remote binds are detected instead of silently producing a dead proxy.

## Git Proxy

When `remoteProxy.apply.gitConfig` is enabled, apply writes both:

- global Git config through `git config --global --replace-all`
- workspace Git config through `git -C <workspace> config --local --replace-all`

Workspace config is applied only for current remote workspace folders that are inside a Git worktree. If the target machine does not have `git`, the extension logs `git not found on remote; skipped Git proxy config` and continues applying VS Code, terminal, and server environment settings.

Use these settings to control the behavior:

- `remoteProxy.apply.gitGlobalConfig`
- `remoteProxy.apply.gitWorkspaceConfig`
- `remoteProxy.apply.gitForceOverride`

## Host Profiles

Use `remoteProxy.hostProfiles` for server-specific overrides. Keys can be the SSH alias, such as `office`, or the resolved target key, such as `alice@ssh.example.com`.

```json
{
  "remoteProxy.hostProfiles": {
    "office": {
      "noProxy": "localhost,127.0.0.1,::1,.cluster.local",
      "applyGitConfig": false
    }
  }
}
```

## Remote Status File

When `remoteProxy.apply.remoteStatusFile` is enabled, the extension writes:

```text
~/.vscode-server/remote-proxy-status.json
```

This file records the active proxy URL, remote port, update time, and local proxy source for diagnostics or shell integrations.
Kernel-mode routes also include backend, route id, selected transport, connect mode, and fallback reason when those fields are available.

## Cleanup

Run `Remote Proxy: Clean Remote Settings` when you want to remove the settings this extension managed on the current SSH host. It stops the owned tunnel, removes the managed `server-env-setup` block, deletes the remote status file, unsets remote global Git proxy values, and removes proxy keys from the remote VS Code machine settings file.

The cleanup command is explicit on purpose: `Remote Proxy: Stop` only stops the current tunnel and leaves proxy defaults in place for the next reconnect.

## Important SSH note

The extension starts a background `ssh -R` process from the local extension host. For this to be non-interactive, your SSH config should use keys, an agent, or an existing control master. By default `remoteProxy.ssh.batchMode` is enabled so the command fails quickly instead of hanging on a password prompt.

By default, the extension writes remote settings through SSH instead of the VS Code configuration API. This avoids accidentally changing local user settings from the local UI extension host.

## Troubleshooting

If status shows `remote: ssh-remote (no authority)`, the extension first asks Remote - SSH's internal `remote-internal.getActiveSshRemote` command for the active host, then checks current workspace storage. Global `storage.json` fallback is disabled by default because it can be stale in Extension Development Host windows. If you previously used `Remote Proxy: Pick SSH Host`, run `Remote Proxy: Clear SSH Host Override` to return to automatic host detection.

## Development

```bash
npm install
npm run compile
npx @vscode/vsce package
```

## Bundled ssh_proxy Kernel

For kernel-mode packages, stage release binaries before packaging:

```powershell
npm run kernel:stage
npm run package:with-kernel
```

The staging script calls `../../scripts/build-release.ps1`, then copies the
Windows x64 binary and Linux x64 musl binary into `assets/bin`. At runtime the
extension uses a non-default explicit `remoteProxy.sshProxy.executable` first,
then a matching bundled binary, then `ssh_proxy` from `PATH`. Set the executable
to an empty string when you want PATH-only discovery.
