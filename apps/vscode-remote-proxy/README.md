# Remote Proxy Auto Forward

Remote Proxy Auto Forward exposes your local proxy inside a VS Code Remote SSH window and applies the matching proxy settings to the remote environment.

The extension is useful when a remote shell, remote extension host, Git, or toolchain must reach the network through a proxy that only exists on your local desktop.

## How It Works

The extension detects a local proxy from:

- `remoteProxy.localProxy.url`
- `HTTP_PROXY`, `HTTPS_PROXY`, or `ALL_PROXY`
- configured local proxy probe ports

It then exposes a remote listener:

```text
remote 127.0.0.1:<remote-port> -> local proxy
```

The listener URL is written to remote VS Code settings, terminal environment variables, optional server-env setup, optional Git config, and a remote status file.

## Daemon Path

The extension uses the bundled or configured `ssh_proxy` binary as a thin daemon client:

```text
VS Code extension
  -> ssh_proxy vscode up
  -> local ssh_proxy daemon
  -> daemon job/readiness/route state
  -> remote 127.0.0.1:<port>
  -> local upstream proxy
```

The daemon owns route ids, job progress, readiness, peer state, update state, remote setup, and health repair. The extension no longer runs the old service/session/OpenSSH fallback chain in the normal path.

Auto-start never prompts for UAC or sudo elevation. Interactive commands can guide the user to install or update the local daemon. OpenSSH is not part of the normal path; it is reserved for explicit compatibility workflows only.

## Daemon Job Readiness

`Remote Proxy: Start` calls `ssh_proxy vscode up` and receives a daemon job id plus the intended remote proxy URL. The extension then polls `ssh_proxy vscode status` and renders the daemon phase instead of starting its own route or service fallback chain.

Typical phases are `resolve_target`, `ensure_remote_peer`, `start_route`, `wait_route_ready`, `verify_remote_port`, `apply_remote_settings`, and `healthy`. A terminal `failed` or `cancelled` job includes a blocker, last error, and next action for Diagnose output.

When no daemon is reachable, auto-start reports the blocker in the status bar.
Interactive `Remote Proxy: Start` offers an `Install Daemon` action, runs
`ssh_proxy daemon install --scope system --elevate --json`, then retries the
daemon session after installation. Background startup does not open an elevation
prompt.

## Quick Start

1. Install the extension or launch it in Extension Development Host.
2. Connect a VS Code window through Remote SSH.
3. Keep `remoteProxy.enabled=true` and `remoteProxy.autoStart=true`.
4. Run `Remote Proxy: Diagnose` if the status bar does not show a running proxy.

Useful commands:

- `Remote Proxy: Start`
- `Remote Proxy: Stop`
- `Remote Proxy: Restart`
- `Remote Proxy: Apply Remote Settings`
- `Remote Proxy: Clean Remote Settings`
- `Remote Proxy: Pick Local Proxy`
- `Remote Proxy: Pick SSH Host`
- `Remote Proxy: Diagnose`
- `Remote Proxy: Show Status`
- `Remote Proxy: Show Output`
- `Remote Proxy: Open Settings`

The status bar menu exposes the same operations.

## Applied Remote State

When enabled, the extension manages:

- Remote VS Code Machine settings: `http.proxy`, `http.proxySupport`, and terminal env.
- `~/.vscode-server/server-env-setup` managed block for remote extension host inheritance.
- Remote Git proxy config, globally and per current workspace Git repo.
- `~/.vscode-server/remote-proxy-status.json` for diagnostics and reuse.

`Remote Proxy: Stop` only stops the active route. `Remote Proxy: Clean Remote Settings` removes managed settings and Git/env/status changes.

## Important Settings

| Setting | Default | Purpose |
| --- | --- | --- |
| `remoteProxy.localProxy.mode` | `auto` | Detect proxy from manual URL, env, then port probes. |
| `remoteProxy.localProxy.url` | empty | Manual local proxy URL. |
| `remoteProxy.remote.port` | `17890` | Preferred remote listener port. |
| `remoteProxy.remote.autoPickPort` | `true` | Try nearby ports if the preferred port is busy. |
| `remoteProxy.sshProxy.executable` | `ssh_proxy` | Explicit binary, bundled binary, or PATH fallback. |
| `remoteProxy.sshProxy.connectMode` | `auto` | Let the daemon prefer the persistent remote peer and fall back when topology requires it. |
| `remoteProxy.forward.verifyAfterStart` | `true` | Verify remote listener readiness after route start. |
| `remoteProxy.forward.healthCheckEnabled` | `true` | Periodically verify the active listener. |
| `remoteProxy.apply.gitConfig` | `true` | Apply remote Git proxy config. |

## Host Profiles

Use `remoteProxy.hostProfiles` for SSH-host specific overrides. Keys can be SSH aliases or resolved target keys.

```json
{
  "remoteProxy.hostProfiles": {
    "office": {
      "localProxyUrl": "http://127.0.0.1:10808/",
      "noProxy": "localhost,127.0.0.1,::1,.cluster.local",
      "applyGitConfig": false
    }
  }
}
```

## Troubleshooting

Run `Remote Proxy: Diagnose` first. It prints the detected SSH host, local proxy, remote proxy URL, daemon job phase, route id, transport, daemon health, route health, blocker, repair action, and latest error, then offers to copy the redacted daemon report.

Common failures:

- `502 Bad Gateway`: the remote listener accepted the request but could not open the upstream path. Check the local proxy URL, including scheme and port, and confirm the local proxy accepts HTTP CONNECT or SOCKS5 traffic.
- `Access is denied` during daemon install: Windows blocked service registration or named-pipe ACL repair. Auto-start will not pop UAC; run an interactive daemon install/update command or inspect `ssh_proxy doctor --json --report`.
- `node_control_token_required`: an old token-backed daemon/config is still running. Use the Diagnose repair action or reinstall the system daemon interactively.
- Remote port already in use: keep `remoteProxy.remote.autoPickPort=true`, or pick a different `remoteProxy.remote.port`.
- Host unresolved in Extension Development Host: run `Remote Proxy: Pick SSH Host`, or enable storage fallback only if you understand it can be stale.
- Route stuck in `accepted`, `bootstrapping_peer`, or `starting`: open output, inspect `ssh_proxy vscode status --workspace <id> --json` and `ssh_proxy events --job <job-id> --json`, and verify remote `127.0.0.1:<port>` reachability.
- Unexpected OpenSSH usage: treat it as an emergency compatibility path. Diagnose should include a daemon reason such as unsupported Rust SSH configuration.

Remote shell smoke test:

```bash
echo "$http_proxy"
curl -I www.google.com
```

## Development

Install dependencies and compile:

```bash
npm install
npm run compile
```

Run tests:

```bash
npm test
```

Launch an Extension Development Host:

```powershell
pwsh -NoProfile -File ..\..\scripts\launch-vscode-extension-dev.ps1
```

Run the local prerelease build, stage the bundled kernel binaries, package the
VSIX, and optionally launch the debug host:

```powershell
pwsh -NoProfile -File ..\..\scripts\prerelease-local.ps1 -LaunchVscode
```

Package without restaging kernel binaries:

```bash
npm run package
```

Package with freshly staged release binaries:

```powershell
npm run package:with-kernel
```

The staging script builds release binaries, then copies the Windows x64 and Linux x64 musl `ssh_proxy` binaries into `assets/bin`.
