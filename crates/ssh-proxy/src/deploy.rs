mod defaults;
mod descriptor;
mod helper;
mod host;
mod install;
mod profile_record;
mod remote_commands;
mod token;
mod transport;

pub(crate) use descriptor::{RemoteDescriptorResult, refresh_remote_peer_descriptor};
pub use host::host;
pub use install::{RemoteInstallResult, install_remote};
pub(crate) use profile_record::{
    record_remote_descriptor_profile, record_remote_install_profile,
    record_remote_token_rotation_profile,
};
pub(crate) use token::{RemoteTokenRotateResult, rotate_remote_peer_token};
pub(crate) use transport::{AutoTransportError, RemoteHelperTimings, TransportCandidateFailure};
pub use transport::{open_remote_helper, open_remote_reverse_socks};

#[cfg(test)]
mod tests {
    use crate::{
        cli, config, peer_lifecycle,
        ssh_client::{self},
    };

    use super::*;
    use crate::peer_lifecycle::{
        service_provider::PeerServiceProvider, workflow::LifecycleOperation,
    };
    use descriptor::{apply_descriptor_to_install_args, descriptor_protocols};
    use host::host_exec_response;
    use profile_record::apply_remote_token_rotation_profile;

    fn install_args(persist: cli::PersistMode) -> cli::InstallRemoteArgs {
        cli::InstallRemoteArgs {
            target: "peer".to_string(),
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

    #[tokio::test]
    async fn remote_install_plan_returns_lifecycle_report() {
        let args = install_args(cli::PersistMode::Systemd);
        let spec = peer_lifecycle::spec::PeerLifecycleSpec::remote_peer(
            "peer",
            "/home/me/bin/ssh_proxy",
            &args,
            peer_lifecycle::service_provider::ServiceProviderKind::SystemdUser,
        );
        let executor = peer_lifecycle::executor::FakeExecutor::default();
        executor.push_output(ssh_client::ExecOutput {
            exit_status: 0,
            stdout: "ok".to_string(),
            stderr: String::new(),
        });

        let provider = peer_lifecycle::service_provider::ServiceProviderPlan::new(
            peer_lifecycle::service_provider::ServiceProviderKind::SystemdUser,
            "ssh-proxy-helper",
        );
        let report = install::run_remote_install_plan(
            &executor,
            &spec,
            provider.lifecycle_plan(&spec, LifecycleOperation::Install, Some("true".to_string())),
        )
        .await
        .unwrap();

        assert_eq!(report["role"], "remote_peer");
        assert_eq!(report["operation"], "install");
        assert_eq!(report["provider"], "systemd_user");
        assert_eq!(executor.commands(), vec!["true"]);
    }

    #[test]
    fn descriptor_updates_install_endpoints() {
        let descriptor = serde_json::json!({
            "endpoints": {
                "control": "tcp://127.0.0.1:29181",
                "transport": "127.0.0.1:29180",
                "tls_transport": "127.0.0.1:29182",
                "quic_transport": "127.0.0.1:29183"
            },
            "transport_protocols": ["quic", "tls-tcp", "plain-tcp"]
        });
        let mut args = cli::InstallRemoteArgs {
            target: "peer".to_string(),
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
            remote_os: cli::RemoteOs::Auto,
            remote_token: None,
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
            persist: cli::PersistMode::None,
        };

        apply_descriptor_to_install_args(&descriptor, &mut args);

        assert_eq!(args.remote_control, "127.0.0.1:29181".parse().unwrap());
        assert_eq!(args.remote_tcp, "127.0.0.1:29180".parse().unwrap());
        assert_eq!(
            args.remote_tls_transport,
            Some("127.0.0.1:29182".parse().unwrap())
        );
        assert_eq!(
            args.remote_quic_transport,
            Some("127.0.0.1:29183".parse().unwrap())
        );
        assert_eq!(
            descriptor_protocols(&descriptor).unwrap(),
            vec!["quic", "tls-tcp", "plain-tcp"]
        );
    }

    #[test]
    fn token_rotation_updates_profile_and_peer_record() {
        let mut config = config::AppConfig::default();
        config.peers.insert(
            "peer".to_string(),
            config::PeerRecord {
                node_id: Some("old-node".to_string()),
                node_name: Some("old-name".to_string()),
                target: Some("old-target".to_string()),
                remote_path: Some("/old/bin/ssh_proxy".to_string()),
                control_endpoint: Some("tcp://127.0.0.1:19081".to_string()),
                transport: Some("127.0.0.1:19080".parse().unwrap()),
                token: Some("old-token".to_string()),
                ..Default::default()
            },
        );
        let result = RemoteTokenRotateResult {
            target: "host".to_string(),
            remote_path: "/home/me/bin/ssh_proxy".to_string(),
            remote_control: "127.0.0.1:29181".parse().unwrap(),
            remote_tcp: "127.0.0.1:29180".parse().unwrap(),
            remote_tls_transport: Some("127.0.0.1:29182".parse().unwrap()),
            remote_quic_transport: None,
            remote_token: "new-token".to_string(),
            token_metadata: Some(config::TokenMetadata::rotated("peer-control-transport", 2)),
            descriptor: Some(serde_json::json!({
                "node_id": "new-node",
                "node_name": "new-name",
                "version": "0.3.0",
                "control_api_version": 1,
                "peer_protocol_version": 1,
                "features": ["frames-v1", "token-auth-v1"],
                "os": "linux",
                "arch": "x86_64",
                "endpoints": {
                    "control": "tcp://127.0.0.1:29181",
                    "transport": "127.0.0.1:29180",
                    "tls_transport": "127.0.0.1:29182"
                },
                "transport_protocols": ["tls-tcp", "plain-tcp"]
            })),
            response: serde_json::json!({"ok": true}),
        };

        apply_remote_token_rotation_profile(&mut config, "peer", &result);

        let profile = config.profiles.get("peer").unwrap();
        assert_eq!(profile.remote_token.as_deref(), Some("new-token"));
        assert_eq!(profile.remote_tls, Some("127.0.0.1:29182".parse().unwrap()));
        let peer = config.peers.get("peer").unwrap();
        assert_eq!(peer.node_id.as_deref(), Some("new-node"));
        assert_eq!(peer.node_name.as_deref(), Some("new-name"));
        assert_eq!(peer.version.as_deref(), Some("0.3.0"));
        assert_eq!(peer.control_api_version, Some(1));
        assert_eq!(peer.peer_protocol_version, Some(1));
        assert_eq!(peer.features, vec!["frames-v1", "token-auth-v1"]);
        assert_eq!(peer.os.as_deref(), Some("linux"));
        assert_eq!(peer.arch.as_deref(), Some("x86_64"));
        assert_eq!(peer.trust.as_deref(), Some("ssh-token-rotate"));
        assert_eq!(peer.token.as_deref(), Some("new-token"));
        assert_eq!(peer.transport_protocols, vec!["tls-tcp", "plain-tcp"]);
        assert_eq!(
            peer.token_metadata.as_ref().unwrap().scope,
            "peer-control-transport"
        );
    }

    #[test]
    fn host_exec_response_has_stable_json_contract() {
        let value = host_exec_response(
            "edge",
            "remote setup",
            Some(7),
            "hello\n",
            "warning\n",
            42,
            false,
        );

        assert_eq!(value["ok"], false);
        assert_eq!(value["kind"], "host_exec");
        assert_eq!(value["target"], "edge");
        assert_eq!(value["label"], "remote setup");
        assert_eq!(value["exit_code"], 7);
        assert_eq!(value["stdout"], "hello\n");
        assert_eq!(value["stderr"], "warning\n");
        assert_eq!(value["duration_ms"], 42);
        assert_eq!(value["timed_out"], false);
    }

    #[test]
    fn host_exec_timeout_response_uses_null_exit_code() {
        let value = host_exec_response(
            "edge",
            "remote setup",
            None,
            "",
            "host exec timed out after 3s",
            3001,
            true,
        );

        assert_eq!(value["ok"], false);
        assert!(value["exit_code"].is_null());
        assert_eq!(value["stderr"], "host exec timed out after 3s");
        assert_eq!(value["timed_out"], true);
    }
}
