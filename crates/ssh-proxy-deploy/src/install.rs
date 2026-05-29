use std::{net::SocketAddr, path::PathBuf};

use ssh_proxy_core::{
    intent::{RemoteInstallIntent, SshTargetIntent},
    model::{PersistenceMode, RemotePlatform},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteInstallPlan {
    pub ssh: SshTargetIntent,
    pub remote_platform: RemotePlatform,
    pub persistence: PersistenceMode,
    pub remote_path: Option<String>,
    pub remote_bin: Option<PathBuf>,
    pub remote_token: Option<String>,
    pub remote_tcp: SocketAddr,
    pub remote_control: SocketAddr,
    pub remote_tls_transport: Option<SocketAddr>,
    pub remote_quic_transport: Option<SocketAddr>,
    pub remote_tls_cert: Option<String>,
    pub remote_tls_key: Option<String>,
    pub remote_tls_client_ca: Option<String>,
}

impl RemoteInstallPlan {
    pub fn from_intent(intent: &RemoteInstallIntent) -> Self {
        Self {
            ssh: intent.ssh.clone(),
            remote_platform: intent.remote_platform,
            persistence: intent.persistence,
            remote_path: intent.remote_path.clone(),
            remote_bin: intent.remote_bin.clone(),
            remote_token: intent.remote_token.clone(),
            remote_tcp: intent.remote_tcp,
            remote_control: intent.remote_control,
            remote_tls_transport: intent.remote_tls_transport,
            remote_quic_transport: intent.remote_quic_transport,
            remote_tls_cert: intent.remote_tls_cert.clone(),
            remote_tls_key: intent.remote_tls_key.clone(),
            remote_tls_client_ca: intent.remote_tls_client_ca.clone(),
        }
    }

    pub fn requires_persistent_service(&self) -> bool {
        self.persistence != PersistenceMode::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_install_plan_keeps_command_neutral_endpoints() {
        let mut intent = RemoteInstallIntent::new(
            SshTargetIntent::new("edge"),
            "127.0.0.1:19080".parse().unwrap(),
            "127.0.0.1:19081".parse().unwrap(),
            PersistenceMode::Systemd,
        );
        intent.remote_platform = RemotePlatform::Unix;
        intent.remote_path = Some("/opt/ssh_proxy/bin/ssh_proxy".to_string());
        intent.remote_tls_transport = Some("127.0.0.1:19443".parse().unwrap());

        let plan = RemoteInstallPlan::from_intent(&intent);

        assert_eq!(plan.ssh.target, "edge");
        assert_eq!(plan.remote_platform, RemotePlatform::Unix);
        assert_eq!(plan.persistence, PersistenceMode::Systemd);
        assert!(plan.requires_persistent_service());
        assert_eq!(plan.remote_tcp.to_string(), "127.0.0.1:19080");
        assert_eq!(
            plan.remote_tls_transport.unwrap().to_string(),
            "127.0.0.1:19443"
        );
    }

    #[test]
    fn transient_remote_install_plan_does_not_require_service() {
        let intent = RemoteInstallIntent::new(
            SshTargetIntent::new("edge"),
            "127.0.0.1:19080".parse().unwrap(),
            "127.0.0.1:19081".parse().unwrap(),
            PersistenceMode::None,
        );

        let plan = RemoteInstallPlan::from_intent(&intent);

        assert!(!plan.requires_persistent_service());
    }
}
