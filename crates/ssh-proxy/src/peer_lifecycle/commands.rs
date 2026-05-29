use std::net::SocketAddr;

use crate::cli;
use ssh_proxy_core::{intent::RemoteInstallIntent, model::RemotePlatform};
use ssh_proxy_lifecycle::service_provider::RemotePeerServiceSpec;

use super::{artifacts::PeerArtifact, provider};

pub(crate) fn remote_write_peer_artifact_command(
    artifact: PeerArtifact,
    remote_os: cli::RemoteOs,
) -> String {
    let remote_platform: RemotePlatform = remote_os.into();
    ssh_proxy_lifecycle::service_provider::remote_write_peer_artifact_command(
        artifact,
        remote_platform,
    )
}

pub(crate) fn remote_auto_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    ssh_proxy_lifecycle::service_provider::remote_auto_install_command(&service_spec(
        remote_path,
        args,
    ))
}

pub(crate) fn remote_systemd_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    ssh_proxy_lifecycle::service_provider::remote_systemd_install_command(&service_spec(
        remote_path,
        args,
    ))
}

pub(crate) fn remote_launchd_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    ssh_proxy_lifecycle::service_provider::remote_launchd_install_command(&service_spec(
        remote_path,
        args,
    ))
}

pub(crate) fn remote_nohup_start_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
    stop_existing: bool,
) -> String {
    ssh_proxy_lifecycle::service_provider::remote_nohup_start_command(
        &service_spec(remote_path, args),
        stop_existing,
    )
}

pub(crate) fn remote_schtasks_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    ssh_proxy_lifecycle::service_provider::remote_schtasks_install_command(&service_spec(
        remote_path,
        args,
    ))
}

pub(crate) fn remote_nohup_status_snippet(remote_tcp: SocketAddr) -> String {
    ssh_proxy_lifecycle::service_provider::remote_nohup_status_snippet(remote_tcp)
}

pub(crate) fn remote_nohup_stop_snippet(remote_tcp: SocketAddr) -> String {
    ssh_proxy_lifecycle::service_provider::remote_nohup_stop_snippet(remote_tcp)
}

pub(crate) fn remote_nohup_files(remote_tcp: SocketAddr) -> (String, String, String, String) {
    ssh_proxy_lifecycle::service_provider::remote_nohup_files(remote_tcp)
}

pub(crate) fn token_arg(token: Option<&str>) -> String {
    provider::token_arg(token)
}

pub(crate) fn node_daemon_extra_args(args: &cli::InstallRemoteArgs) -> String {
    provider::node_daemon_extra_args(args)
}

pub(crate) fn sh_quote(value: &str) -> String {
    provider::sh_quote(value)
}

fn service_spec(remote_path: &str, args: &cli::InstallRemoteArgs) -> RemotePeerServiceSpec {
    let intent: RemoteInstallIntent = args.into();
    RemotePeerServiceSpec::from_intent(remote_path, &intent)
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

        assert!(systemd.contains("systemctl --user daemon-reload"));
        assert!(launchd.contains("launchctl bootstrap"));
        assert!(nohup.contains("nohup /bin/sh"));
        assert!(!systemd.contains("\"service_manager\""));
        assert!(!launchd.contains("\"service_manager\""));
        assert!(!nohup.contains("\"service_manager\""));
    }

    #[test]
    fn windows_schtasks_command_uses_user_task() {
        let mut args = install_args(cli::PersistMode::Schtasks);
        args.remote_os = cli::RemoteOs::Windows;
        let command =
            remote_schtasks_install_command(r"%LOCALAPPDATA%\ssh_proxy\bin\ssh_proxy.exe", &args);

        assert!(command.contains("schtasks /Create"));
        assert!(!command.contains("windows_schtasks_user"));
    }
}
