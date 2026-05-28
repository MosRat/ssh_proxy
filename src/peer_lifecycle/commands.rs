use std::net::SocketAddr;

use crate::cli;

use super::artifacts::PeerArtifact;

pub(crate) fn remote_write_peer_artifact_command(
    artifact: PeerArtifact,
    remote_os: cli::RemoteOs,
) -> String {
    let name = artifact.file_name();
    match remote_os {
        cli::RemoteOs::Windows => {
            let preserve_existing = if artifact.preserve_existing() {
                "if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $tmp -Force; exit 0 }; "
            } else {
                ""
            };
            format!(
                "powershell -NoProfile -ExecutionPolicy Bypass -Command \"$ErrorActionPreference='Stop'; $home=[Environment]::GetFolderPath('UserProfile'); $dir=Join-Path $home '.ssh_proxy'; New-Item -ItemType Directory -Force -Path $dir | Out-Null; $p=Join-Path $dir '{name}'; $tmp=Join-Path $dir ('{name}.tmp.'+[Guid]::NewGuid().ToString('N')); $fs=[IO.File]::Open($tmp,'CreateNew','Write','None'); [Console]::OpenStandardInput().CopyTo($fs); $fs.Close(); {preserve_existing}Move-Item -LiteralPath $tmp -Destination $p -Force\""
            )
        }
        cli::RemoteOs::Unix | cli::RemoteOs::Auto => {
            let preserve_existing = if artifact.preserve_existing() {
                "[ -f \"$p\" ] && { rm -f \"$tmp\"; exit 0; }; "
            } else {
                ""
            };
            let backup_existing = if artifact.backup_existing() {
                "if [ -f \"$p\" ]; then cp \"$p\" \"$HOME/.ssh_proxy/config.toml.bak\" 2>/dev/null || true; fi; "
            } else {
                ""
            };
            format!(
                "set -eu; mkdir -p \"$HOME/.ssh_proxy\"; p=\"$HOME/.ssh_proxy/{name}\"; tmp=\"$p.tmp.$$\"; umask 077; cat > \"$tmp\"; {preserve_existing}{backup_existing}mv \"$tmp\" \"$p\"; chmod 600 \"$p\" 2>/dev/null || true"
            )
        }
    }
}

pub(crate) fn remote_auto_install_command(
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

pub(crate) fn remote_systemd_install_command(
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

pub(crate) fn remote_launchd_install_command(
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

pub(crate) fn remote_nohup_start_command(
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

pub(crate) fn remote_schtasks_install_command(
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

pub(crate) fn remote_nohup_status_snippet(remote_tcp: SocketAddr) -> String {
    let (pidfile, childfile, logfile, scriptfile) = remote_nohup_files(remote_tcp);
    format!(
        "pidfile={}; childfile={}; logfile={}; scriptfile={}; if [ -f \"$pidfile\" ]; then pid=$(cat \"$pidfile\" 2>/dev/null || true); if [ -n \"$pid\" ] && kill -0 \"$pid\" 2>/dev/null; then echo \"supervisor running pid=$pid\"; else echo \"stale supervisor pidfile pid=$pid\"; fi; else echo \"not installed\"; fi; if [ -f \"$childfile\" ]; then child=$(cat \"$childfile\" 2>/dev/null || true); if [ -n \"$child\" ] && kill -0 \"$child\" 2>/dev/null; then echo \"helper running pid=$child\"; else echo \"stale helper child pid=$child\"; fi; fi; if [ -f \"$logfile\" ]; then echo \"log=$logfile\"; tail -n 20 \"$logfile\" 2>/dev/null || true; fi",
        pidfile, childfile, logfile, scriptfile
    )
}

pub(crate) fn remote_nohup_stop_snippet(remote_tcp: SocketAddr) -> String {
    let (pidfile, childfile, _, _) = remote_nohup_files(remote_tcp);
    format!(
        "pidfile={}; childfile={}; for f in \"$childfile\" \"$pidfile\"; do if [ -f \"$f\" ]; then pid=$(cat \"$f\" 2>/dev/null || true); if [ -n \"$pid\" ] && kill -0 \"$pid\" 2>/dev/null; then kill \"$pid\" 2>/dev/null || true; for i in 1 2 3 4 5; do kill -0 \"$pid\" 2>/dev/null || break; sleep 1; done; kill -9 \"$pid\" 2>/dev/null || true; fi; rm -f \"$f\"; fi; done",
        pidfile, childfile
    )
}

pub(crate) fn remote_nohup_files(remote_tcp: SocketAddr) -> (String, String, String, String) {
    let port = remote_tcp.port();
    (
        format!("$HOME/.ssh_proxy/run/helper-{port}.pid"),
        format!("$HOME/.ssh_proxy/run/helper-{port}.child.pid"),
        format!("$HOME/.ssh_proxy/log/helper-{port}.log"),
        format!("$HOME/.ssh_proxy/run/helper-{port}.supervisor.sh"),
    )
}

pub(crate) fn token_arg(token: Option<&str>) -> String {
    token
        .map(|token| format!(" --token {}", sh_quote(token)))
        .unwrap_or_default()
}

pub(crate) fn node_daemon_extra_args(args: &cli::InstallRemoteArgs) -> String {
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

pub(crate) fn remote_mark_service_state_command(manager: &str, state: &str, phase: &str) -> String {
    format!(
        "now=$(date +%s 2>/dev/null || printf 0); mkdir -p \"$HOME/.ssh_proxy\"; cat > \"$HOME/.ssh_proxy/install_report.json\" <<EOF\n{{\"schema\":\"ssh_proxy_remote_install.v1\",\"state\":\"{state}\",\"phase\":\"{phase}\",\"service_manager\":\"{manager}\",\"updated_at_unix\":$now}}\nEOF\ncat > \"$HOME/.ssh_proxy/health.json\" <<EOF\n{{\"schema\":\"ssh_proxy_peer_health.v1\",\"state\":\"{state}\",\"service_manager\":\"{manager}\",\"updated_at_unix\":$now}}\nEOF"
    )
}

pub(crate) fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
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
    fn write_artifact_command_preserves_routes_and_backs_up_config() {
        let routes = remote_write_peer_artifact_command(PeerArtifact::Routes, cli::RemoteOs::Unix);
        let config = remote_write_peer_artifact_command(PeerArtifact::Config, cli::RemoteOs::Unix);

        assert!(routes.contains("[ -f \"$p\" ] && { rm -f \"$tmp\"; exit 0; }"));
        assert!(config.contains("config.toml.bak"));
        assert!(!routes.contains("config.toml.bak"));
    }

    #[test]
    fn provider_commands_render_stable_service_managers() {
        let args = install_args(cli::PersistMode::Systemd);
        let systemd = remote_systemd_install_command("/home/me/bin/ssh_proxy", &args);
        let launchd = remote_launchd_install_command("/Users/me/bin/ssh_proxy", &args);
        let nohup = remote_nohup_start_command("/home/me/bin/ssh_proxy", &args, true);

        assert!(systemd.contains("\"service_manager\":\"systemd_user\""));
        assert!(launchd.contains("\"service_manager\":\"launchd_user\""));
        assert!(nohup.contains("\"service_manager\":\"nohup_supervisor\""));
    }

    #[test]
    fn windows_schtasks_command_uses_user_task() {
        let mut args = install_args(cli::PersistMode::Schtasks);
        args.remote_os = cli::RemoteOs::Windows;
        let command =
            remote_schtasks_install_command(r"%LOCALAPPDATA%\ssh_proxy\bin\ssh_proxy.exe", &args);

        assert!(command.contains("schtasks /Create"));
        assert!(command.contains("windows_schtasks_user"));
    }
}
