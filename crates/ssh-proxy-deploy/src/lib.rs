use std::net::SocketAddr;

use serde_json::Value;
use ssh_proxy_config::TokenMetadata;

mod commands;
mod install;
mod remote_admin;
mod remote_setup;
mod remote_setup_scripts;

pub use commands::{
    default_persistent_remote_path_command, remote_clean_command, remote_doctor_command,
    remote_logs_command, remote_node_control_command, remote_node_control_json_command,
    remote_resolve_peer_defaults_command, remote_restart_command, remote_status_command,
    remote_stop_command, sh_quote,
};
pub use install::RemoteInstallPlan;
pub use remote_admin::{
    RemoteAdminChecksumReport, RemoteAdminDefaultsReport, RemoteAdminGitConfigReport,
    RemoteAdminIntent, apply_git_proxy_config, cleanup_git_proxy_config, remote_admin_error,
    remote_admin_ok, remote_admin_stdin_command,
};
pub use remote_setup::{
    RemoteArtifactIntent, RemoteArtifactKind, RemoteSetupExecutionPlan, RemoteSetupPayloadInput,
    RemoteSetupPlan, RemoteSetupScriptIntent, build_proxy_env, build_remote_setup_payload,
};
pub use remote_setup_scripts::{
    build_cleanup_script_with_git, build_git_config_script, build_server_env_setup_content,
};

#[derive(Debug, Clone)]
pub struct RemoteInstallResult {
    pub target: String,
    pub remote_node_id: Option<String>,
    pub remote_node_name: Option<String>,
    pub remote_path: String,
    pub service_manager: String,
    pub remote_tcp: SocketAddr,
    pub remote_control: SocketAddr,
    pub remote_tls_transport: Option<SocketAddr>,
    pub remote_quic_transport: Option<SocketAddr>,
    pub remote_token: Option<String>,
    pub descriptor: Option<Value>,
    pub install_report: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct RemoteDescriptorResult {
    pub target: String,
    pub remote_path: String,
    pub remote_control: SocketAddr,
    pub remote_tcp: SocketAddr,
    pub remote_tls_transport: Option<SocketAddr>,
    pub remote_quic_transport: Option<SocketAddr>,
    pub remote_token: Option<String>,
    pub descriptor: Value,
}

#[derive(Debug, Clone)]
pub struct RemoteTokenRotateResult {
    pub target: String,
    pub remote_path: String,
    pub remote_control: SocketAddr,
    pub remote_tcp: SocketAddr,
    pub remote_tls_transport: Option<SocketAddr>,
    pub remote_quic_transport: Option<SocketAddr>,
    pub remote_token: String,
    pub token_metadata: Option<TokenMetadata>,
    pub descriptor: Option<Value>,
    pub response: Value,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn remote_install_result_keeps_endpoint_fields() {
        let result = RemoteInstallResult {
            target: "host".to_string(),
            remote_node_id: Some("node".to_string()),
            remote_node_name: None,
            remote_path: "/tmp/ssh_proxy".to_string(),
            service_manager: "systemd-user".to_string(),
            remote_tcp: "127.0.0.1:19080".parse().unwrap(),
            remote_control: "127.0.0.1:19081".parse().unwrap(),
            remote_tls_transport: Some("127.0.0.1:19082".parse().unwrap()),
            remote_quic_transport: Some("127.0.0.1:19083".parse().unwrap()),
            remote_token: Some("token".to_string()),
            descriptor: Some(json!({"ok": true})),
            install_report: None,
        };

        assert_eq!(result.remote_tcp.port(), 19080);
        assert_eq!(result.remote_control.port(), 19081);
        assert_eq!(result.descriptor.as_ref().unwrap()["ok"], true);
    }
}
