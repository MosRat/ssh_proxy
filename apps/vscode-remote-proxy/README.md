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

## Backends

`remoteProxy.backend=auto` is the default.

Kernel mode uses the bundled or configured `ssh_proxy` binary:

```text
remote tools -> remote 127.0.0.1:<port>
  -> remote ssh_proxy listener
  -> reverse-link/direct route
  -> local ssh_proxy daemon or session daemon
  -> local upstream proxy
```

OpenSSH mode is the compatibility fallback:

```text
ssh -R remote:port:local-proxy-host:local-proxy-port
```

Kernel mode is preferred because it exposes route ids, ownership, session-daemon fallback, structured health, and richer diagnostics. If kernel startup hits a service or route failure, the extension now falls back to the session daemon first and then OpenSSH before surfacing a hard failure.

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

## Multi-window Reuse

`remoteProxy.singleton.reuseEnabled=true` lets compatible VS Code windows connected to the same SSH target share one proxy route.

The owner writes a local lease heartbeat. Other windows reuse the remote listener. If the owner exits or the route becomes unreachable, another window can take over after the lease or health checks fail.

Remote port selection is sticky. The extension tries the current route, remembered port, lease port, remote status file port, and configured range before giving up.

## Important Settings

| Setting | Default | Purpose |
| --- | --- | --- |
| `remoteProxy.backend` | `auto` | Prefer `ssh_proxy`, fall back to OpenSSH. |
| `remoteProxy.localProxy.mode` | `auto` | Detect proxy from manual URL, env, then port probes. |
| `remoteProxy.localProxy.url` | empty | Manual local proxy URL. |
| `remoteProxy.remote.port` | `17890` | Preferred remote listener port. |
| `remoteProxy.remote.autoPickPort` | `true` | Try nearby ports if the preferred port is busy. |
| `remoteProxy.sshProxy.executable` | `ssh_proxy` | Explicit binary, bundled binary, or PATH fallback. |
| `remoteProxy.sshProxy.autoInstallLocalService` | `true` | Probe existing service scopes first, then repair/install before session daemon fallback. |
| `remoteProxy.sshProxy.connectMode` | `reverse-link` | Preserve `ssh -R` style reachability by default. |
| `remoteProxy.sshProxy.remoteSetup` | `auto` | Prefer `ssh_proxy host exec`, fallback to OpenSSH. |
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

Run `Remote Proxy: Diagnose` first. It prints backend, detected SSH host, lease state, local proxy, remote proxy URL, route id, transport, fallback reason, daemon health, route health, and the latest error.

Common failures:

- `502 Bad Gateway`: the remote listener accepted the request but could not open the upstream path. Check the local proxy URL, including scheme and port, and confirm the local proxy accepts HTTP CONNECT or SOCKS5 traffic.
- `Access is denied` during service install: Windows blocked scheduled task/service registration. The kernel path will cache that failure for the session, fall back to the session daemon, and then try OpenSSH if needed.
- Remote port already in use: keep `remoteProxy.remote.autoPickPort=true`, or pick a different `remoteProxy.remote.port`.
- Host unresolved in Extension Development Host: run `Remote Proxy: Pick SSH Host`, or enable storage fallback only if you understand it can be stale.
- Route stuck in `starting`: open output, inspect `ssh_proxy node control routes`, and verify remote `127.0.0.1:<port>` reachability.

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
code --new-window --extensionDevelopmentPath=F:\WorkSpace\Rust\ssh_proxy\apps\vscode-remote-proxy
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
