use std::net::SocketAddr;

use anyhow::Result;
use serde_json::Value;
use ssh_proxy_core::{intent::RemoteInstallIntent, model::RemotePlatform};
use ssh_proxy_lifecycle::service_provider::RemotePeerServiceSpec;

#[cfg(test)]
use crate::peer_lifecycle::commands;
use crate::{cli, ssh_client};

pub(super) fn remote_status_command(remote_path: &str, remote_tcp: SocketAddr) -> String {
    ssh_proxy_deploy::remote_status_command(remote_path, remote_tcp)
}

pub(super) fn remote_node_control_command(
    remote_path: &str,
    remote_control: SocketAddr,
    remote_token: Option<&str>,
    control_args: &str,
) -> String {
    ssh_proxy_deploy::remote_node_control_command(
        remote_path,
        remote_control,
        remote_token,
        control_args,
    )
}

pub(super) fn remote_node_control_json_command(
    remote_path: &str,
    remote_control: SocketAddr,
    remote_token: Option<&str>,
    request: &Value,
) -> String {
    ssh_proxy_deploy::remote_node_control_json_command(
        remote_path,
        remote_control,
        remote_token,
        request,
    )
}

pub(super) async fn default_persistent_remote_path(
    client: &ssh_client::Client,
    remote_os: cli::RemoteOs,
) -> Result<String> {
    if remote_os == cli::RemoteOs::Windows {
        return Ok(r"%LOCALAPPDATA%\ssh_proxy\bin\ssh_proxy.exe".to_string());
    }
    let output = client
        .exec_output(ssh_proxy_deploy::default_persistent_remote_path_command())
        .await?;
    Ok(output.trim().to_string())
}

pub(super) fn remote_resolve_peer_defaults_command(
    preferred_transport: SocketAddr,
    preferred_control: SocketAddr,
    remote_os: cli::RemoteOs,
) -> String {
    let remote_platform: RemotePlatform = remote_os.into();
    ssh_proxy_deploy::remote_resolve_peer_defaults_command(
        preferred_transport,
        preferred_control,
        remote_platform,
    )
}

pub(super) fn remote_stop_command(remote_tcp: SocketAddr) -> String {
    ssh_proxy_deploy::remote_stop_command(remote_tcp)
}

pub(super) fn remote_restart_command(remote_path: &str, args: &cli::InstallRemoteArgs) -> String {
    ssh_proxy_deploy::remote_restart_command(&service_spec(remote_path, args))
}

pub(super) fn remote_logs_command(remote_tcp: SocketAddr, lines: usize) -> String {
    ssh_proxy_deploy::remote_logs_command(remote_tcp, lines)
}

pub(super) fn remote_clean_command(remote_path: &str, remote_tcp: SocketAddr) -> String {
    ssh_proxy_deploy::remote_clean_command(remote_path, remote_tcp)
}

pub(super) fn remote_doctor_command(remote_path: &str, remote_tcp: SocketAddr) -> String {
    ssh_proxy_deploy::remote_doctor_command(remote_path, remote_tcp)
}

pub(super) fn sh_quote(value: &str) -> String {
    ssh_proxy_deploy::sh_quote(value)
}

fn service_spec(remote_path: &str, args: &cli::InstallRemoteArgs) -> RemotePeerServiceSpec {
    let intent: RemoteInstallIntent = args.into();
    RemotePeerServiceSpec::from_intent(remote_path, &intent)
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

        let command = commands::remote_systemd_install_command("/home/me/bin/ssh_proxy", &args);

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
        assert!(!command.contains("\"service_manager\""));
    }

    #[test]
    fn remote_launchd_install_uses_keepalive() {
        let args = install_args(cli::PersistMode::Launchd);

        let command = commands::remote_launchd_install_command("/Users/me/bin/ssh_proxy", &args);

        assert!(command.contains("com.ssh-proxy.helper.plist"), "{command}");
        assert!(command.contains("<key>KeepAlive</key><true/>"), "{command}");
        assert!(command.contains("launchctl bootstrap"), "{command}");
        assert!(!command.contains("\"service_manager\""));
    }

    #[test]
    fn remote_schtasks_install_uses_user_task() {
        let mut args = install_args(cli::PersistMode::Schtasks);
        args.remote_os = cli::RemoteOs::Windows;

        let command = commands::remote_schtasks_install_command(
            r"%LOCALAPPDATA%\ssh_proxy\bin\ssh_proxy.exe",
            &args,
        );

        assert!(command.contains("schtasks /Create"), "{command}");
        assert!(command.contains("/TN ssh_proxy_helper"), "{command}");
        assert!(!command.contains("\"service_manager"));
        assert!(!command.contains("windows_schtasks_user"));
    }
}
