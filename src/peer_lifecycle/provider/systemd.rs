use crate::cli;

use super::util::{node_daemon_extra_args, remote_mark_service_state_command, sh_quote, token_arg};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_command_uses_foreground_node_daemon() {
        let args = cli::InstallRemoteArgs {
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
            persist: cli::PersistMode::Systemd,
        };
        let command = remote_systemd_install_command("/home/me/bin/ssh_proxy", &args);

        assert!(command.contains("ExecStart='/home/me/bin/ssh_proxy' node daemon"));
        assert!(command.contains("Restart=always"));
        assert!(command.contains("\"service_manager\":\"systemd_user\""));
    }
}
