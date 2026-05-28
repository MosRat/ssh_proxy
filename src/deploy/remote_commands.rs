use std::net::SocketAddr;

use anyhow::Result;
use serde_json::Value;

use crate::{cli, ssh_client};

pub(super) fn remote_status_command(remote_path: &str, remote_tcp: SocketAddr) -> String {
    let version_check = format!("{} --version 2>/dev/null || true", sh_quote(remote_path));
    format!(
        "set +e; printf 'remote_binary_version='; {version_check}; \
             printf 'peer_state=\\n'; cat \"$HOME/.ssh_proxy/peer_state.json\" 2>/dev/null || true; printf '\\n'; \
             printf 'install_report=\\n'; cat \"$HOME/.ssh_proxy/install_report.json\" 2>/dev/null || true; printf '\\n'; \
             printf 'health=\\n'; cat \"$HOME/.ssh_proxy/health.json\" 2>/dev/null || true; printf '\\n'; \
             printf 'launchd_user_status=\\n'; launchctl print gui/$(id -u)/com.ssh-proxy.helper 2>&1 || true; \
             printf 'systemd_user_status=\\n'; systemctl --user status --no-pager ssh-proxy-helper.service 2>&1 || true; \
             printf '\\nnohup_status=\\n'; {status}",
        status = remote_nohup_status_snippet(remote_tcp)
    )
}

pub(super) fn remote_node_control_command(
    remote_path: &str,
    remote_control: SocketAddr,
    remote_token: Option<&str>,
    control_args: &str,
) -> String {
    let token_arg = remote_token
        .map(|token| format!(" --token {}", sh_quote(token)))
        .unwrap_or_default();
    format!(
        "set -eu; {} node control --endpoint tcp://{}{} {}",
        sh_quote(remote_path),
        remote_control,
        token_arg,
        control_args
    )
}

pub(super) fn remote_node_control_json_command(
    remote_path: &str,
    remote_control: SocketAddr,
    remote_token: Option<&str>,
    request: &Value,
) -> String {
    let request = request.to_string();
    remote_node_control_command(
        remote_path,
        remote_control,
        remote_token,
        &format!("send {}", sh_quote(&request)),
    )
}

pub(super) async fn default_persistent_remote_path(
    client: &ssh_client::Client,
    remote_os: cli::RemoteOs,
) -> Result<String> {
    if remote_os == cli::RemoteOs::Windows {
        return Ok(r"%LOCALAPPDATA%\ssh_proxy\bin\ssh_proxy.exe".to_string());
    }
    let command = format!(
        "set -eu; home=\"${{HOME:-/tmp}}\"; base=\"\"; for d in \"$home/.local/bin\" \"$home/bin\" \"$home/.ssh_proxy/bin\"; do case \":${{PATH:-}}:\" in *\":$d:\"*) base=\"$d\"; break;; esac; done; if [ -z \"$base\" ]; then base=\"$home/.local/bin\"; fi; mkdir -p \"$base\"; case \":${{PATH:-}}:\" in *\":$base:\"*) ;; *) profile=\"$home/.profile\"; touch \"$profile\" 2>/dev/null && if ! grep -q 'ssh_proxy managed PATH' \"$profile\" 2>/dev/null; then printf '\\n# ssh_proxy managed PATH\\ncase \":$PATH:\" in *\":$HOME/.local/bin:\"*) ;; *) export PATH=\"$HOME/.local/bin:$PATH\";; esac\\n' >> \"$profile\"; fi || true;; esac; printf '%s' \"$base/ssh_proxy\""
    );
    let output = client.exec_output(command).await?;
    Ok(output.trim().to_string())
}

pub(super) fn remote_auto_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    format!(
        "set -eu; if [ \"$(uname -s 2>/dev/null || true)\" = Darwin ] && command -v launchctl >/dev/null 2>&1; then {launchd}; elif command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then {systemd}; else {nohup}; fi",
        launchd = remote_launchd_install_command(remote_path, args),
        systemd = remote_systemd_install_command(remote_path, args),
        nohup = remote_nohup_start_command(remote_path, args, true)
    )
}

pub(super) fn remote_write_config_command_for_os(
    preferred_transport: SocketAddr,
    preferred_control: SocketAddr,
    token: &str,
    local_node_id: Option<&str>,
    local_node_name: Option<&str>,
    local_control_endpoint: Option<&str>,
    local_transport: Option<SocketAddr>,
    remote_os: cli::RemoteOs,
) -> String {
    match remote_os {
        cli::RemoteOs::Windows => remote_write_config_powershell_command(
            preferred_transport,
            preferred_control,
            token,
            local_node_id,
            local_node_name,
            local_control_endpoint,
            local_transport,
        ),
        cli::RemoteOs::Unix | cli::RemoteOs::Auto => remote_write_config_command(
            preferred_transport,
            preferred_control,
            token,
            local_node_id,
            local_node_name,
            local_control_endpoint,
            local_transport,
        ),
    }
}

pub(super) fn remote_write_config_command(
    preferred_transport: SocketAddr,
    preferred_control: SocketAddr,
    token: &str,
    local_node_id: Option<&str>,
    local_node_name: Option<&str>,
    local_control_endpoint: Option<&str>,
    local_transport: Option<SocketAddr>,
) -> String {
    let peer_table = local_node_id
            .map(|node_id| {
                let node_name = toml_quote(local_node_name.unwrap_or("local"));
                let control = local_control_endpoint
                    .map(toml_quote)
                    .map(|value| format!("control_endpoint = {value}\n"))
                    .unwrap_or_default();
                let transport = local_transport
                    .map(|addr| toml_quote(&addr.to_string()))
                    .map(|value| format!("transport = {value}\n"))
                    .unwrap_or_default();
                format!(
                    "\n[peers.bootstrap-local]\nnode_id = {node_id}\nnode_name = {node_name}\ntrust = \"ssh-bootstrap\"\n{control}{transport}",
                    node_id = toml_quote(node_id),
                )
            })
            .unwrap_or_default();
    format!(
        r#"set -eu
mkdir -p "$HOME/.ssh_proxy"
is_free() {{
  port="$1"
  if command -v ss >/dev/null 2>&1; then
    ! ss -ltn 2>/dev/null | grep -Eq "[.:]${{port}}[[:space:]]"
  elif command -v netstat >/dev/null 2>&1; then
    ! netstat -ltn 2>/dev/null | grep -Eq "[.:]${{port}}[[:space:]]"
  else
    return 0
  fi
}}
pick_port() {{
  start="$1"
  end=$((start + 199))
  port="$start"
  while [ "$port" -le "$end" ]; do
    if is_free "$port"; then printf '%s' "$port"; return 0; fi
    port=$((port + 1))
  done
  printf '%s' "$start"
}}
transport_port=$(pick_port {transport_port})
control_port=$(pick_port {control_port})
transport="127.0.0.1:$transport_port"
control="127.0.0.1:$control_port"
config_file="$HOME/.ssh_proxy/config.toml"
existing_node_id=""
existing_node_name=""
if [ -f "$config_file" ]; then
  existing_node_id="$(sed -n 's/^node_id = "\(.*\)"$/\1/p' "$config_file" 2>/dev/null | head -n 1 || true)"
  existing_node_name="$(sed -n 's/^node_name = "\(.*\)"$/\1/p' "$config_file" 2>/dev/null | head -n 1 || true)"
fi
node_name="$(id -un 2>/dev/null || printf unknown)@$(hostname 2>/dev/null || printf unknown)"
node_id=""
if command -v od >/dev/null 2>&1; then
  node_id="spx-$(od -An -N32 -tx1 /dev/urandom 2>/dev/null | tr -d ' \n')"
fi
if [ -z "$node_id" ]; then
  node_id="spx-$(date +%s)-$$"
fi
if [ -n "$existing_node_id" ]; then node_id="$existing_node_id"; fi
if [ -n "$existing_node_name" ]; then node_name="$existing_node_name"; fi
created_at_unix="$(date +%s 2>/dev/null || printf 0)"
if [ -f "$config_file" ]; then
  cp "$config_file" "$HOME/.ssh_proxy/config.toml.bak" 2>/dev/null || true
fi
cat > "$config_file" <<EOF
[identity]
node_id = "$node_id"
node_name = "$node_name"
secret = {token}

[daemon]
control_endpoint = "tcp://$control"
transport_listen = "$transport"
token = {token}
route_autostart = true

[daemon.token_metadata]
created_at_unix = $created_at_unix
scope = "daemon-control-transport"
{peer_table}
EOF
chmod 600 "$config_file" 2>/dev/null || true
cat > "$HOME/.ssh_proxy/peer_state.json" <<EOF
{{"schema":"ssh_proxy_peer_state.v1","state":"configured","service_manager":"pending","transport":"$transport","control":"tcp://$control","updated_at_unix":$created_at_unix}}
EOF
cat > "$HOME/.ssh_proxy/install_report.json" <<EOF
{{"schema":"ssh_proxy_remote_install.v1","state":"configured","phase":"write_config","service_manager":"pending","updated_at_unix":$created_at_unix}}
EOF
cat > "$HOME/.ssh_proxy/health.json" <<EOF
{{"schema":"ssh_proxy_peer_health.v1","state":"starting","transport":"$transport","control":"tcp://$control","updated_at_unix":$created_at_unix}}
EOF
[ -f "$HOME/.ssh_proxy/routes.json" ] || printf '{{"version":1,"routes":[]}}\n' > "$HOME/.ssh_proxy/routes.json"
printf 'transport=%s\ncontrol=%s\nnode_id=%s\nnode_name=%s\nconfig=%s\n' "$transport" "$control" "$node_id" "$node_name" "$config_file"
"#,
        transport_port = preferred_transport.port(),
        control_port = preferred_control.port(),
        token = toml_quote(token),
        peer_table = peer_table
    )
}

fn remote_write_config_powershell_command(
    preferred_transport: SocketAddr,
    preferred_control: SocketAddr,
    token: &str,
    local_node_id: Option<&str>,
    local_node_name: Option<&str>,
    local_control_endpoint: Option<&str>,
    local_transport: Option<SocketAddr>,
) -> String {
    let token = ps_single_quote(token);
    let peer_table = local_node_id
        .map(|node_id| {
            let node_name = local_node_name.unwrap_or("local");
            let control = local_control_endpoint
                .map(|value| format!("control_endpoint = {value:?}`n"))
                .unwrap_or_default();
            let transport = local_transport
                .map(|addr| format!("transport = {:?}`n", addr.to_string()))
                .unwrap_or_default();
            format!(
                "`n[peers.bootstrap-local]`nnode_id = {node_id:?}`nnode_name = {node_name:?}`ntrust = \"ssh-bootstrap\"`n{control}{transport}"
            )
        })
        .unwrap_or_default();
    format!(
        "powershell -NoProfile -ExecutionPolicy Bypass -Command \"$ErrorActionPreference='Stop'; $home=[Environment]::GetFolderPath('UserProfile'); $dir=Join-Path $home '.ssh_proxy'; New-Item -ItemType Directory -Force -Path $dir | Out-Null; $transport='127.0.0.1:{transport_port}'; $control='127.0.0.1:{control_port}'; $config=Join-Path $dir 'config.toml'; $nodeId='spx-'+[Guid]::NewGuid().ToString('N'); $nodeName=\\\"$env:USERNAME@$env:COMPUTERNAME\\\"; $created=[DateTimeOffset]::UtcNow.ToUnixTimeSeconds(); $token={token}; $content=\\\"[identity]`nnode_id = \\\\\\\"$nodeId\\\\\\\"`nnode_name = \\\\\\\"$nodeName\\\\\\\"`nsecret = \\\\\\\"$token\\\\\\\"`n`n[daemon]`ncontrol_endpoint = \\\\\\\"tcp://$control\\\\\\\"`ntransport_listen = \\\\\\\"$transport\\\\\\\"`ntoken = \\\\\\\"$token\\\\\\\"`nroute_autostart = true`n`n[daemon.token_metadata]`ncreated_at_unix = $created`nscope = \\\\\\\"daemon-control-transport\\\\\\\"`n{peer_table}\\\"; Set-Content -LiteralPath $config -Value $content -Encoding UTF8; $peerState=Join-Path $dir 'peer_state.json'; $install=Join-Path $dir 'install_report.json'; $health=Join-Path $dir 'health.json'; $routes=Join-Path $dir 'routes.json'; Set-Content -LiteralPath $peerState -Value \\\"{{`\\\\\\\"schema`\\\\\\\":`\\\\\\\"ssh_proxy_peer_state.v1`\\\\\\\",`\\\\\\\"state`\\\\\\\":`\\\\\\\"configured`\\\\\\\",`\\\\\\\"service_manager`\\\\\\\":`\\\\\\\"pending`\\\\\\\",`\\\\\\\"transport`\\\\\\\":`\\\\\\\"$transport`\\\\\\\",`\\\\\\\"control`\\\\\\\":`\\\\\\\"tcp://$control`\\\\\\\",`\\\\\\\"updated_at_unix`\\\\\\\":$created}}\\\"; Set-Content -LiteralPath $install -Value \\\"{{`\\\\\\\"schema`\\\\\\\":`\\\\\\\"ssh_proxy_remote_install.v1`\\\\\\\",`\\\\\\\"state`\\\\\\\":`\\\\\\\"configured`\\\\\\\",`\\\\\\\"phase`\\\\\\\":`\\\\\\\"write_config`\\\\\\\",`\\\\\\\"service_manager`\\\\\\\":`\\\\\\\"pending`\\\\\\\",`\\\\\\\"updated_at_unix`\\\\\\\":$created}}\\\"; Set-Content -LiteralPath $health -Value \\\"{{`\\\\\\\"schema`\\\\\\\":`\\\\\\\"ssh_proxy_peer_health.v1`\\\\\\\",`\\\\\\\"state`\\\\\\\":`\\\\\\\"starting`\\\\\\\",`\\\\\\\"transport`\\\\\\\":`\\\\\\\"$transport`\\\\\\\",`\\\\\\\"control`\\\\\\\":`\\\\\\\"tcp://$control`\\\\\\\",`\\\\\\\"updated_at_unix`\\\\\\\":$created}}\\\"; if (!(Test-Path -LiteralPath $routes)) {{ Set-Content -LiteralPath $routes -Value '{{\\\"version\\\":1,\\\"routes\\\":[]}}' }}; Write-Output \\\"transport=$transport\\\"; Write-Output \\\"control=$control\\\"; Write-Output \\\"node_id=$nodeId\\\"; Write-Output \\\"node_name=$nodeName\\\"; Write-Output \\\"config=$config\\\"\"",
        transport_port = preferred_transport.port(),
        control_port = preferred_control.port(),
        token = token,
        peer_table = peer_table.replace('"', "`\\\""),
    )
}

pub(super) fn remote_systemd_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    let escaped = sh_quote(remote_path);
    let token_arg = token_arg(args.remote_token.as_deref());
    let extra_args = node_daemon_extra_args(args);
    format!(
        "set -eu; systemctl --user show-environment >/dev/null; if command -v loginctl >/dev/null 2>&1; then loginctl enable-linger \"$(id -un)\" >/dev/null 2>&1 || true; fi; mkdir -p ~/.config/systemd/user; cat > ~/.config/systemd/user/ssh-proxy-helper.service <<'EOF'\n[Unit]\nDescription=ssh_proxy node daemon\nAfter=network-online.target\nWants=network-online.target\nStartLimitIntervalSec=0\n[Service]\nExecStart={} node daemon --transport {} --control tcp://{}{}{}\nRestart=always\nRestartSec=3\nKillSignal=SIGINT\n[Install]\nWantedBy=default.target\nEOF\nsystemctl --user daemon-reload && systemctl --user enable ssh-proxy-helper.service && systemctl --user restart ssh-proxy-helper.service && {}",
        escaped,
        args.remote_tcp,
        args.remote_control,
        token_arg,
        extra_args,
        remote_mark_service_state_command("systemd_user", "healthy", "start_service")
    )
}

pub(super) fn remote_launchd_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    let token_arg = token_arg(args.remote_token.as_deref());
    let extra_args = node_daemon_extra_args(args);
    let plist = "$HOME/Library/LaunchAgents/com.ssh-proxy.helper.plist";
    format!(
        "set -eu; mkdir -p \"$HOME/Library/LaunchAgents\"; cat > {plist} <<'EOF'\n<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>com.ssh-proxy.helper</string>\n<key>ProgramArguments</key><array><string>{remote_path}</string><string>node</string><string>daemon</string><string>--transport</string><string>{remote_tcp}</string><string>--control</string><string>tcp://{remote_control}</string>{token_plist}{extra_plist}</array>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><true/>\n<key>StandardOutPath</key><string>{home}/.ssh_proxy/log/launchd.log</string>\n<key>StandardErrorPath</key><string>{home}/.ssh_proxy/log/launchd.log</string>\n</dict></plist>\nEOF\nmkdir -p \"$HOME/.ssh_proxy/log\"; launchctl bootout gui/$(id -u) {plist} >/dev/null 2>&1 || true; launchctl bootstrap gui/$(id -u) {plist}; launchctl kickstart -k gui/$(id -u)/com.ssh-proxy.helper; {mark}",
        plist = plist,
        remote_path = xml_escape(remote_path),
        remote_tcp = args.remote_tcp,
        remote_control = args.remote_control,
        token_plist = token_arg_to_plist(&token_arg),
        extra_plist = extra_args_to_plist(&extra_args),
        home = "$HOME",
        mark = remote_mark_service_state_command("launchd_user", "healthy", "start_service"),
    )
}

pub(super) fn remote_nohup_start_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
    stop_existing: bool,
) -> String {
    let stop = if stop_existing {
        format!("{}; ", remote_nohup_stop_snippet(args.remote_tcp))
    } else {
        String::new()
    };
    let token_arg = token_arg(args.remote_token.as_deref());
    let extra_args = node_daemon_extra_args(args);
    let (pidfile, childfile, logfile, scriptfile) = remote_nohup_files(args.remote_tcp);
    format!(
        "set -eu; mkdir -p \"$HOME/.ssh_proxy/run\" \"$HOME/.ssh_proxy/log\"; {stop} cat > {scriptfile} <<'EOF'\n#!/bin/sh\nset -u\ntrap 'if [ -f {childfile} ]; then child=$(cat {childfile} 2>/dev/null || true); [ -n \"$child\" ] && kill \"$child\" 2>/dev/null || true; fi; exit 0' INT TERM\nbackoff=1\nwhile :; do\n  echo \"$(date -u +%Y-%m-%dT%H:%M:%SZ) starting ssh_proxy node daemon\" >>{logfile}\n  {remote_path} node daemon --transport {remote_tcp} --control tcp://{remote_control}{token_arg}{extra_args} >>{logfile} 2>&1 &\n  child=$!\n  echo \"$child\" >{childfile}\n  wait \"$child\"\n  code=$?\n  echo \"$(date -u +%Y-%m-%dT%H:%M:%SZ) ssh_proxy node daemon exited status=$code; restarting in ${{backoff}}s\" >>{logfile}\n  sleep \"$backoff\"\n  if [ \"$backoff\" -lt 30 ]; then backoff=$((backoff * 2)); fi\ndone\nEOF\nchmod 700 {scriptfile}; nohup /bin/sh {scriptfile} >/dev/null 2>&1 < /dev/null & echo $! > {pidfile}; sleep 1; pid=$(cat {pidfile}); kill -0 \"$pid\"; {mark}",
        remote_path = sh_quote(remote_path),
        remote_tcp = args.remote_tcp,
        remote_control = args.remote_control,
        token_arg = token_arg,
        extra_args = extra_args,
        logfile = logfile,
        pidfile = pidfile,
        childfile = childfile,
        scriptfile = scriptfile,
        mark = remote_mark_service_state_command("nohup_supervisor", "healthy", "start_service"),
    )
}

pub(super) fn remote_schtasks_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    let token_arg = args
        .remote_token
        .as_deref()
        .map(|token| format!(" --token {}", windows_cmd_quote(token)))
        .unwrap_or_default();
    let command = format!(
        "{} node daemon --transport {} --control tcp://{}{}{}",
        windows_cmd_quote(remote_path),
        args.remote_tcp,
        args.remote_control,
        token_arg,
        windows_extra_args(args)
    );
    format!(
        "schtasks /Create /TN ssh_proxy_helper /SC ONLOGON /RL LIMITED /F /TR {task} && schtasks /Run /TN ssh_proxy_helper && powershell -NoProfile -ExecutionPolicy Bypass -Command \"$dir=Join-Path ([Environment]::GetFolderPath('UserProfile')) '.ssh_proxy'; New-Item -ItemType Directory -Force -Path $dir | Out-Null; $now=[DateTimeOffset]::UtcNow.ToUnixTimeSeconds(); Set-Content -LiteralPath (Join-Path $dir 'install_report.json') -Value \\\"{{`\\\\\\\"schema`\\\\\\\":`\\\\\\\"ssh_proxy_remote_install.v1`\\\\\\\",`\\\\\\\"state`\\\\\\\":`\\\\\\\"healthy`\\\\\\\",`\\\\\\\"phase`\\\\\\\":`\\\\\\\"start_service`\\\\\\\",`\\\\\\\"service_manager`\\\\\\\":`\\\\\\\"windows_schtasks_user`\\\\\\\",`\\\\\\\"updated_at_unix`\\\\\\\":$now}}\\\"\"",
        task = windows_cmd_quote(&command),
    )
}

pub(super) fn remote_stop_command(remote_tcp: SocketAddr) -> String {
    format!(
        "set +e; systemctl --user stop ssh-proxy-helper.service >/dev/null 2>&1; {}; echo stopped",
        remote_nohup_stop_snippet(remote_tcp)
    )
}

pub(super) fn remote_restart_command(remote_path: &str, args: &cli::InstallRemoteArgs) -> String {
    format!(
        "set -eu; if systemctl --user status ssh-proxy-helper.service >/dev/null 2>&1; then systemctl --user restart ssh-proxy-helper.service; else {}; fi",
        remote_nohup_start_command(remote_path, args, true)
    )
}

pub(super) fn remote_logs_command(remote_tcp: SocketAddr, lines: usize) -> String {
    let (_, _, logfile, _) = remote_nohup_files(remote_tcp);
    let lines = lines.clamp(1, 5000);
    format!(
        "set +e; logfile={logfile}; journalctl --user -u ssh-proxy-helper.service -n {lines} --no-pager 2>/dev/null || true; if [ -f \"$logfile\" ]; then echo '--- nohup log ---'; tail -n {lines} \"$logfile\"; else echo \"no nohup log at $logfile\"; fi",
    )
}

pub(super) fn remote_clean_command(remote_path: &str, remote_tcp: SocketAddr) -> String {
    let (_, childfile, logfile, scriptfile) = remote_nohup_files(remote_tcp);
    format!(
        "set +e; {stop}; for pid in $(pgrep -f {remote_path_pattern} 2>/dev/null || true); do if [ \"$pid\" != \"$$\" ] && [ \"$pid\" != \"$PPID\" ]; then kill \"$pid\" >/dev/null 2>&1 || true; fi; done; systemctl --user disable --now ssh-proxy-helper.service >/dev/null 2>&1; rm -f ~/.config/systemd/user/ssh-proxy-helper.service; systemctl --user daemon-reload >/dev/null 2>&1; rm -f {remote_path} {logfile} {childfile} {scriptfile}; echo cleaned",
        stop = remote_stop_command(remote_tcp),
        remote_path_pattern = sh_quote(&format!("{remote_path} node daemon")),
        remote_path = sh_quote(remote_path),
        logfile = logfile,
        childfile = childfile,
        scriptfile = scriptfile,
    )
}

pub(super) fn remote_doctor_command(remote_path: &str, remote_tcp: SocketAddr) -> String {
    let (pidfile, childfile, logfile, scriptfile) = remote_nohup_files(remote_tcp);
    format!(
        "set +e; echo 'ssh_proxy remote doctor'; echo user=$(id -un 2>/dev/null); echo uid=$(id -u 2>/dev/null); echo home=$HOME; echo shell=$SHELL; echo path=$PATH; echo uname=$(uname -a 2>/dev/null); echo pid1=$(ps -p 1 -o comm= 2>/dev/null); echo systemctl=$(command -v systemctl 2>/dev/null); echo nohup=$(command -v nohup 2>/dev/null); echo ss=$(command -v ss 2>/dev/null); echo remote_tcp={remote_tcp}; echo remote_path={remote_path}; if [ -x {remote_path_q} ]; then {remote_path_q} --version 2>/dev/null || true; else echo binary=missing-or-not-executable; fi; echo remote_path_on_path=$(command -v ssh_proxy 2>/dev/null || true); echo systemd_user_probe=$(systemctl --user show-environment >/dev/null 2>&1; echo $?); echo pidfile={pidfile}; echo childfile={childfile}; echo logfile={logfile}; echo scriptfile={scriptfile}; {status}; if command -v ss >/dev/null 2>&1; then ss -ltnp 2>/dev/null | grep ':{port} ' || true; fi",
        remote_path = remote_path,
        remote_path_q = sh_quote(remote_path),
        status = remote_nohup_status_snippet(remote_tcp),
        port = remote_tcp.port(),
        childfile = childfile,
        scriptfile = scriptfile,
    )
}

pub(super) fn remote_nohup_status_snippet(remote_tcp: SocketAddr) -> String {
    let (pidfile, childfile, logfile, scriptfile) = remote_nohup_files(remote_tcp);
    format!(
        "pidfile={}; childfile={}; logfile={}; scriptfile={}; if [ -f \"$pidfile\" ]; then pid=$(cat \"$pidfile\" 2>/dev/null || true); if [ -n \"$pid\" ] && kill -0 \"$pid\" 2>/dev/null; then echo \"supervisor running pid=$pid\"; else echo \"stale supervisor pidfile pid=$pid\"; fi; else echo \"not installed\"; fi; if [ -f \"$childfile\" ]; then child=$(cat \"$childfile\" 2>/dev/null || true); if [ -n \"$child\" ] && kill -0 \"$child\" 2>/dev/null; then echo \"helper running pid=$child\"; else echo \"stale helper child pid=$child\"; fi; fi; if [ -f \"$logfile\" ]; then echo \"log=$logfile\"; tail -n 20 \"$logfile\" 2>/dev/null || true; fi",
        pidfile, childfile, logfile, scriptfile
    )
}

pub(super) fn remote_nohup_stop_snippet(remote_tcp: SocketAddr) -> String {
    let (pidfile, childfile, _, _) = remote_nohup_files(remote_tcp);
    format!(
        "pidfile={}; childfile={}; for f in \"$childfile\" \"$pidfile\"; do if [ -f \"$f\" ]; then pid=$(cat \"$f\" 2>/dev/null || true); if [ -n \"$pid\" ] && kill -0 \"$pid\" 2>/dev/null; then kill \"$pid\" 2>/dev/null || true; for i in 1 2 3 4 5; do kill -0 \"$pid\" 2>/dev/null || break; sleep 1; done; kill -9 \"$pid\" 2>/dev/null || true; fi; rm -f \"$f\"; fi; done",
        pidfile, childfile
    )
}

pub(super) fn remote_nohup_files(remote_tcp: SocketAddr) -> (String, String, String, String) {
    let port = remote_tcp.port();
    (
        format!("$HOME/.ssh_proxy/run/helper-{port}.pid"),
        format!("$HOME/.ssh_proxy/run/helper-{port}.child.pid"),
        format!("$HOME/.ssh_proxy/log/helper-{port}.log"),
        format!("$HOME/.ssh_proxy/run/helper-{port}.supervisor.sh"),
    )
}

pub(super) fn token_arg(token: Option<&str>) -> String {
    token
        .map(|token| format!(" --token {}", sh_quote(token)))
        .unwrap_or_default()
}

pub(super) fn node_daemon_extra_args(args: &cli::InstallRemoteArgs) -> String {
    let mut out = String::new();
    if let Some(addr) = args.remote_tls_transport {
        out.push_str(&format!(" --tls-transport {addr}"));
    }
    if let Some(addr) = args.remote_quic_transport {
        out.push_str(&format!(" --quic-transport {addr}"));
    }
    if let Some(path) = &args.remote_tls_cert {
        out.push_str(&format!(" --tls-cert {}", sh_quote(path)));
    }
    if let Some(path) = &args.remote_tls_key {
        out.push_str(&format!(" --tls-key {}", sh_quote(path)));
    }
    if let Some(path) = &args.remote_tls_client_ca {
        out.push_str(&format!(" --tls-client-ca {}", sh_quote(path)));
    }
    out
}

fn remote_mark_service_state_command(manager: &str, state: &str, phase: &str) -> String {
    format!(
        "now=$(date +%s 2>/dev/null || printf 0); mkdir -p \"$HOME/.ssh_proxy\"; cat > \"$HOME/.ssh_proxy/install_report.json\" <<EOF\n{{\"schema\":\"ssh_proxy_remote_install.v1\",\"state\":\"{state}\",\"phase\":\"{phase}\",\"service_manager\":\"{manager}\",\"updated_at_unix\":$now}}\nEOF\ncat > \"$HOME/.ssh_proxy/health.json\" <<EOF\n{{\"schema\":\"ssh_proxy_peer_health.v1\",\"state\":\"{state}\",\"service_manager\":\"{manager}\",\"updated_at_unix\":$now}}\nEOF"
    )
}

fn token_arg_to_plist(token_arg: &str) -> String {
    if token_arg.is_empty() {
        return String::new();
    }
    token_arg
        .split_whitespace()
        .map(|part| format!("<string>{}</string>", xml_escape(part.trim_matches('\''))))
        .collect::<Vec<_>>()
        .join("")
}

fn extra_args_to_plist(extra_args: &str) -> String {
    extra_args
        .split_whitespace()
        .map(|part| format!("<string>{}</string>", xml_escape(part.trim_matches('\''))))
        .collect::<Vec<_>>()
        .join("")
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn ps_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn windows_cmd_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}

fn windows_extra_args(args: &cli::InstallRemoteArgs) -> String {
    let mut out = String::new();
    if let Some(addr) = args.remote_tls_transport {
        out.push_str(&format!(" --tls-transport {addr}"));
    }
    if let Some(addr) = args.remote_quic_transport {
        out.push_str(&format!(" --quic-transport {addr}"));
    }
    out
}

pub(super) fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(super) fn toml_quote(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install_args(persist: cli::PersistMode) -> cli::InstallRemoteArgs {
        cli::InstallRemoteArgs {
            target: "edge".to_string(),
            ssh_args: Vec::new(),
            ssh_command: None,
            user: None,
            port: None,
            identity: Vec::new(),
            config: None,
            known_hosts: None,
            accept_new: false,
            insecure_ignore_host_key: false,
            jump: Vec::new(),
            remote_path: None,
            remote_bin: None,
            remote_os: cli::RemoteOs::Unix,
            remote_token: Some("secret".to_string()),
            remote_tcp: "127.0.0.1:19080".parse().unwrap(),
            remote_control: "127.0.0.1:19081".parse().unwrap(),
            local_node_id: None,
            local_node_name: None,
            local_control_endpoint: None,
            local_transport: None,
            remote_node_id: None,
            remote_node_name: None,
            remote_tls_transport: None,
            remote_quic_transport: None,
            remote_tls_cert: None,
            remote_tls_key: None,
            remote_tls_client_ca: None,
            persist,
        }
    }

    #[test]
    fn remote_node_control_command_includes_explicit_token() {
        let command = remote_node_control_command(
            "/home/me/bin/ssh_proxy",
            "127.0.0.1:19081".parse().unwrap(),
            Some("secret token"),
            "status",
        );

        assert!(command.contains("--token 'secret token'"), "{command}");
        assert!(command.contains("node control --endpoint tcp://127.0.0.1:19081"));
    }

    #[test]
    fn remote_node_control_json_command_uses_send_with_token() {
        let request = serde_json::json!({"cmd": "routes"});
        let command = remote_node_control_json_command(
            "/home/me/bin/ssh_proxy",
            "127.0.0.1:19081".parse().unwrap(),
            Some("secret token"),
            &request,
        );

        assert!(command.contains("--token 'secret token'"), "{command}");
        assert!(command.contains(" send '"), "{command}");
    }

    #[test]
    fn remote_systemd_install_restarts_existing_service() {
        let args = install_args(cli::PersistMode::Systemd);

        let command = remote_systemd_install_command("/home/me/bin/ssh_proxy", &args);

        assert!(
            command.contains("systemctl --user daemon-reload"),
            "{command}"
        );
        assert!(
            command.contains("systemctl --user enable ssh-proxy-helper.service"),
            "{command}"
        );
        assert!(
            command.contains("systemctl --user restart ssh-proxy-helper.service"),
            "{command}"
        );
        assert!(
            !command.contains("enable --now ssh-proxy-helper.service"),
            "{command}"
        );
        assert!(command.contains("\"service_manager\":\"systemd_user\""));
    }

    #[test]
    fn remote_launchd_install_uses_keepalive() {
        let args = install_args(cli::PersistMode::Launchd);

        let command = remote_launchd_install_command("/Users/me/bin/ssh_proxy", &args);

        assert!(command.contains("com.ssh-proxy.helper.plist"), "{command}");
        assert!(command.contains("<key>KeepAlive</key><true/>"), "{command}");
        assert!(command.contains("launchctl bootstrap"), "{command}");
        assert!(command.contains("\"service_manager\":\"launchd_user\""));
    }

    #[test]
    fn remote_schtasks_install_uses_user_task() {
        let mut args = install_args(cli::PersistMode::Schtasks);
        args.remote_os = cli::RemoteOs::Windows;

        let command =
            remote_schtasks_install_command(r"%LOCALAPPDATA%\ssh_proxy\bin\ssh_proxy.exe", &args);

        assert!(command.contains("schtasks /Create"), "{command}");
        assert!(command.contains("/TN ssh_proxy_helper"), "{command}");
        assert!(command.contains("\"service_manager"));
        assert!(command.contains("windows_schtasks_user"));
    }
}
