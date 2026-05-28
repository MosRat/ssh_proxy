use std::net::SocketAddr;

use anyhow::Result;
use serde_json::Value;

use crate::{cli, peer_lifecycle::commands, ssh_client};

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

pub(super) fn remote_resolve_peer_defaults_command(
    preferred_transport: SocketAddr,
    preferred_control: SocketAddr,
    remote_os: cli::RemoteOs,
) -> String {
    match remote_os {
        cli::RemoteOs::Windows => format!(
            "powershell -NoProfile -ExecutionPolicy Bypass -Command \"$ErrorActionPreference='Stop'; $home=[Environment]::GetFolderPath('UserProfile'); $dir=Join-Path $home '.ssh_proxy'; New-Item -ItemType Directory -Force -Path $dir | Out-Null; $config=Join-Path $dir 'config.toml'; $nodeId=''; $nodeName=\\\"$env:USERNAME@$env:COMPUTERNAME\\\"; if (Test-Path -LiteralPath $config) {{ Get-Content -LiteralPath $config | ForEach-Object {{ if ($_ -match '^node_id = \\\\\\\"([^\\\\\\\"]+)\\\\\\\"') {{ $nodeId=$Matches[1] }}; if ($_ -match '^node_name = \\\\\\\"([^\\\\\\\"]+)\\\\\\\"') {{ $nodeName=$Matches[1] }} }} }}; Write-Output \\\"transport=127.0.0.1:{transport_port}\\\"; Write-Output \\\"control=127.0.0.1:{control_port}\\\"; Write-Output \\\"node_id=$nodeId\\\"; Write-Output \\\"node_name=$nodeName\\\"; Write-Output \\\"config=$config\\\"\"",
            transport_port = preferred_transport.port(),
            control_port = preferred_control.port(),
        ),
        cli::RemoteOs::Unix | cli::RemoteOs::Auto => format!(
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

pub(super) fn remote_systemd_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    commands::remote_systemd_install_command(remote_path, args)
}

pub(super) fn remote_launchd_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    commands::remote_launchd_install_command(remote_path, args)
}

pub(super) fn remote_nohup_start_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
    stop_existing: bool,
) -> String {
    commands::remote_nohup_start_command(remote_path, args, stop_existing)
}

pub(super) fn remote_schtasks_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    commands::remote_schtasks_install_command(remote_path, args)
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
    commands::remote_nohup_status_snippet(remote_tcp)
}

pub(super) fn remote_nohup_stop_snippet(remote_tcp: SocketAddr) -> String {
    commands::remote_nohup_stop_snippet(remote_tcp)
}

pub(super) fn remote_nohup_files(remote_tcp: SocketAddr) -> (String, String, String, String) {
    commands::remote_nohup_files(remote_tcp)
}

pub(super) fn sh_quote(value: &str) -> String {
    commands::sh_quote(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer_lifecycle::artifacts::PeerArtifact;

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
    fn remote_config_write_uses_stdin_file_upload_contract() {
        let command =
            commands::remote_write_peer_artifact_command(PeerArtifact::Config, cli::RemoteOs::Unix);

        assert!(command.contains("cat > \"$tmp\""), "{command}");
        assert!(command.contains("config.toml.bak"), "{command}");
        assert!(!command.contains("[identity]"), "{command}");
        assert!(!command.contains("node_id ="), "{command}");
    }

    #[test]
    fn remote_routes_write_preserves_existing_routes_file() {
        let command =
            commands::remote_write_peer_artifact_command(PeerArtifact::Routes, cli::RemoteOs::Unix);

        assert!(
            command.contains("[ -f \"$p\" ] && { rm -f \"$tmp\"; exit 0; }"),
            "{command}"
        );
    }

    #[test]
    fn remote_resolve_defaults_only_reports_runtime_values() {
        let command = remote_resolve_peer_defaults_command(
            "127.0.0.1:19080".parse().unwrap(),
            "127.0.0.1:19081".parse().unwrap(),
            cli::RemoteOs::Unix,
        );

        assert!(command.contains("pick_port 19080"), "{command}");
        assert!(command.contains("printf 'transport=%s"), "{command}");
        assert!(!command.contains("cat > \"$config_file\""), "{command}");
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
