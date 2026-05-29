use std::net::SocketAddr;

use serde_json::Value;
use ssh_proxy_core::model::RemotePlatform;
use ssh_proxy_lifecycle::service_provider::{
    RemotePeerServiceSpec, remote_nohup_files, remote_nohup_start_command,
    remote_nohup_status_snippet, remote_nohup_stop_snippet,
};

pub fn remote_status_command(remote_path: &str, remote_tcp: SocketAddr) -> String {
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

pub fn remote_node_control_command(
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

pub fn remote_node_control_json_command(
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

pub fn default_persistent_remote_path_command() -> String {
    "set -eu; home=\"${HOME:-/tmp}\"; base=\"\"; for d in \"$home/.local/bin\" \"$home/bin\" \"$home/.ssh_proxy/bin\"; do case \":${PATH:-}:\" in *\":$d:\"*) base=\"$d\"; break;; esac; done; if [ -z \"$base\" ]; then base=\"$home/.local/bin\"; fi; mkdir -p \"$base\"; case \":${PATH:-}:\" in *\":$base:\"*) ;; *) profile=\"$home/.profile\"; touch \"$profile\" 2>/dev/null && if ! grep -q 'ssh_proxy managed PATH' \"$profile\" 2>/dev/null; then printf '\\n# ssh_proxy managed PATH\\ncase \":$PATH:\" in *\":$HOME/.local/bin:\"*) ;; *) export PATH=\"$HOME/.local/bin:$PATH\";; esac\\n' >> \"$profile\"; fi || true;; esac; printf '%s' \"$base/ssh_proxy\"".to_string()
}

pub fn remote_resolve_peer_defaults_command(
    preferred_transport: SocketAddr,
    preferred_control: SocketAddr,
    remote_platform: RemotePlatform,
) -> String {
    match remote_platform {
        RemotePlatform::Windows => format!(
            "powershell -NoProfile -ExecutionPolicy Bypass -Command \"$ErrorActionPreference='Stop'; $home=[Environment]::GetFolderPath('UserProfile'); $dir=Join-Path $home '.ssh_proxy'; New-Item -ItemType Directory -Force -Path $dir | Out-Null; $config=Join-Path $dir 'config.toml'; $nodeId=''; $nodeName=\\\"$env:USERNAME@$env:COMPUTERNAME\\\"; if (Test-Path -LiteralPath $config) {{ Get-Content -LiteralPath $config | ForEach-Object {{ if ($_ -match '^node_id = \\\\\\\"([^\\\\\\\"]+)\\\\\\\"') {{ $nodeId=$Matches[1] }}; if ($_ -match '^node_name = \\\\\\\"([^\\\\\\\"]+)\\\\\\\"') {{ $nodeName=$Matches[1] }} }} }}; Write-Output \\\"transport=127.0.0.1:{transport_port}\\\"; Write-Output \\\"control=127.0.0.1:{control_port}\\\"; Write-Output \\\"node_id=$nodeId\\\"; Write-Output \\\"node_name=$nodeName\\\"; Write-Output \\\"config=$config\\\"\"",
            transport_port = preferred_transport.port(),
            control_port = preferred_control.port(),
        ),
        RemotePlatform::Unix | RemotePlatform::Auto => format!(
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
if [ -n "$existing_node_id" ]; then node_id="$existing_node_id"; fi
if [ -n "$existing_node_name" ]; then node_name="$existing_node_name"; fi
printf 'transport=%s\ncontrol=%s\nnode_id=%s\nnode_name=%s\nconfig=%s\n' "$transport" "$control" "$node_id" "$node_name" "$config_file"
"#,
            transport_port = preferred_transport.port(),
            control_port = preferred_control.port(),
        ),
    }
}

pub fn remote_stop_command(remote_tcp: SocketAddr) -> String {
    format!(
        "set +e; systemctl --user stop ssh-proxy-helper.service >/dev/null 2>&1; {}; echo stopped",
        remote_nohup_stop_snippet(remote_tcp)
    )
}

pub fn remote_restart_command(spec: &RemotePeerServiceSpec) -> String {
    format!(
        "set -eu; if systemctl --user status ssh-proxy-helper.service >/dev/null 2>&1; then systemctl --user restart ssh-proxy-helper.service; else {}; fi",
        remote_nohup_start_command(spec, true)
    )
}

pub fn remote_logs_command(remote_tcp: SocketAddr, lines: usize) -> String {
    let (_, _, logfile, _) = remote_nohup_files(remote_tcp);
    let lines = lines.clamp(1, 5000);
    format!(
        "set +e; logfile={logfile}; journalctl --user -u ssh-proxy-helper.service -n {lines} --no-pager 2>/dev/null || true; if [ -f \"$logfile\" ]; then echo '--- nohup log ---'; tail -n {lines} \"$logfile\"; else echo \"no nohup log at $logfile\"; fi",
    )
}

pub fn remote_clean_command(remote_path: &str, remote_tcp: SocketAddr) -> String {
    let (_, childfile, logfile, scriptfile) = remote_nohup_files(remote_tcp);
    format!(
        "set +e; {stop}; for pid in $(pgrep -f {remote_path_pattern} 2>/dev/null || true); do if [ \"$pid\" != \"$$\" ] && [ \"$pid\" != \"$PPID\" ]; then kill \"$pid\" >/dev/null 2>&1 || true; fi; done; systemctl --user disable --now ssh-proxy-helper.service >/dev/null 2>&1; rm -f ~/.config/systemd/user/ssh-proxy-helper.service; systemctl --user daemon-reload >/dev/null 2>&1; rm -f {remote_path} {logfile} {childfile} {scriptfile}; echo cleaned",
        stop = remote_stop_command(remote_tcp),
        remote_path_pattern = sh_quote(&format!("{remote_path} node daemon")),
        remote_path = sh_quote(remote_path),
    )
}

pub fn remote_doctor_command(remote_path: &str, remote_tcp: SocketAddr) -> String {
    let (pidfile, childfile, logfile, scriptfile) = remote_nohup_files(remote_tcp);
    format!(
        "set +e; echo 'ssh_proxy remote doctor'; echo user=$(id -un 2>/dev/null); echo uid=$(id -u 2>/dev/null); echo home=$HOME; echo shell=$SHELL; echo path=$PATH; echo uname=$(uname -a 2>/dev/null); echo pid1=$(ps -p 1 -o comm= 2>/dev/null); echo systemctl=$(command -v systemctl 2>/dev/null); echo nohup=$(command -v nohup 2>/dev/null); echo ss=$(command -v ss 2>/dev/null); echo remote_tcp={remote_tcp}; echo remote_path={remote_path}; if [ -x {remote_path_q} ]; then {remote_path_q} --version 2>/dev/null || true; else echo binary=missing-or-not-executable; fi; echo remote_path_on_path=$(command -v ssh_proxy 2>/dev/null || true); echo systemd_user_probe=$(systemctl --user show-environment >/dev/null 2>&1; echo $?); echo pidfile={pidfile}; echo childfile={childfile}; echo logfile={logfile}; echo scriptfile={scriptfile}; {status}; if command -v ss >/dev/null 2>&1; then ss -ltnp 2>/dev/null | grep ':{port} ' || true; fi",
        remote_path = remote_path,
        remote_path_q = sh_quote(remote_path),
        status = remote_nohup_status_snippet(remote_tcp),
        port = remote_tcp.port(),
    )
}

pub fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn remote_resolve_defaults_only_reports_runtime_values() {
        let command = remote_resolve_peer_defaults_command(
            "127.0.0.1:19080".parse().unwrap(),
            "127.0.0.1:19081".parse().unwrap(),
            RemotePlatform::Unix,
        );

        assert!(command.contains("pick_port 19080"), "{command}");
        assert!(command.contains("printf 'transport=%s"), "{command}");
        assert!(!command.contains("cat > \"$config_file\""), "{command}");
    }
}
