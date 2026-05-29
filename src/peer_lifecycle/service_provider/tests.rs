use super::*;
use crate::{
    cli,
    peer_lifecycle::{
        spec::PeerLifecycleSpec,
        workflow::{LifecycleOperation, PeerLifecyclePhase},
    },
};

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
fn provider_contract_builds_lifecycle_plan() {
    let provider = ServiceProviderPlan::new(ServiceProviderKind::SystemdUser, "ssh_proxy");
    let args = install_args(cli::PersistMode::Systemd);
    let spec = PeerLifecycleSpec::remote_peer(
        "edge",
        "/home/me/bin/ssh_proxy",
        &args,
        ServiceProviderKind::SystemdUser,
    );

    let plan = provider.lifecycle_plan(
        &spec,
        LifecycleOperation::Install,
        Some("systemctl --user restart ssh_proxy".to_string()),
    );

    assert_eq!(provider.kind(), ServiceProviderKind::SystemdUser);
    assert_eq!(provider.service_name(), "ssh_proxy");
    assert_eq!(provider.dependency_report()[0].state, "selected_provider");
    assert_eq!(plan.operation, LifecycleOperation::Install);
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(plan.steps[0].phase, PeerLifecyclePhase::DependencyCheck);
    assert_eq!(plan.steps[1].phase, PeerLifecyclePhase::InstallService);
}

#[test]
fn provider_status_classification_is_stable() {
    let provider = ServiceProviderPlan::new(ServiceProviderKind::SystemdUser, "ssh_proxy");

    let healthy = provider.classify_status(0, "ActiveState=active", "");
    let missing = provider.classify_status(3, "LoadState=not-found", "");
    let denied = provider.classify_status(1, "", "permission denied");

    assert_eq!(healthy.state, ProviderStatusState::Healthy);
    assert_eq!(missing.state, ProviderStatusState::Missing);
    assert_eq!(denied.state, ProviderStatusState::PermissionDenied);
    assert!(provider.repair_hint("service_missing").is_some());
}

#[test]
fn remote_install_plan_preserves_auto_reporting_contract() {
    let args = install_args(cli::PersistMode::Auto);

    let plan = remote_service_install_plan("/home/me/bin/ssh_proxy", &args);

    assert_eq!(plan.provider.kind, ServiceProviderKind::SystemdUser);
    assert_eq!(plan.reported_service_manager, "auto");
    assert!(plan.command.contains("systemctl --user"));
    assert!(plan.command.contains("nohup"));
}
