use super::*;
use crate::cli;

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
fn remote_install_plan_preserves_auto_reporting_contract() {
    let args = install_args(cli::PersistMode::Auto);
    let intent: ssh_proxy_core::intent::RemoteInstallIntent = (&args).into();

    let plan = remote_service_install_plan("/home/me/bin/ssh_proxy", &intent);

    assert_eq!(plan.provider.kind, ServiceProviderKind::SystemdUser);
    assert_eq!(plan.reported_service_manager, "auto");
    assert!(plan.command.contains("systemctl --user"));
    assert!(plan.command.contains("nohup"));
}
