use std::net::SocketAddr;

use serde::{Deserialize, Serialize};
use ssh_proxy_core::{
    intent::RemoteInstallIntent,
    model::{PersistenceMode, RemotePlatform},
};

use crate::service_provider::ServiceProviderKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerLifecycleRole {
    LocalDaemon,
    RemotePeer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerLifecyclePlatform {
    Windows,
    Linux,
    Macos,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerLifecycleScope {
    User,
    System,
    Managed,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackPolicy {
    None,
    PreserveExisting,
    StopAndRestore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerLifecycleSpec {
    pub role: PeerLifecycleRole,
    pub target: String,
    pub platform: PeerLifecyclePlatform,
    pub scope: PeerLifecycleScope,
    pub provider: ServiceProviderKind,
    pub service_name: String,
    pub binary_path: String,
    pub transport: Option<SocketAddr>,
    pub control_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    pub state_dir: String,
    pub rollback_policy: RollbackPolicy,
}

impl PeerLifecycleSpec {
    pub fn remote_peer_from_intent(
        target: impl Into<String>,
        remote_path: impl Into<String>,
        intent: &RemoteInstallIntent,
        provider: ServiceProviderKind,
    ) -> Self {
        Self {
            role: PeerLifecycleRole::RemotePeer,
            target: target.into(),
            platform: platform_from_remote_install(intent.remote_platform, intent.persistence),
            scope: scope_from_persistence(intent.persistence),
            provider,
            service_name: "ssh-proxy-helper".to_string(),
            binary_path: remote_path.into(),
            transport: Some(intent.remote_tcp),
            control_endpoint: Some(format!("tcp://{}", intent.remote_control)),
            token: intent.remote_token.clone(),
            state_dir: remote_state_dir(intent.remote_platform),
            rollback_policy: RollbackPolicy::PreserveExisting,
        }
    }

    pub fn local_daemon(
        target: impl Into<String>,
        binary_path: impl Into<String>,
        provider: ServiceProviderKind,
        service_name: impl Into<String>,
        control_endpoint: Option<String>,
        transport: Option<SocketAddr>,
        token: Option<String>,
        state_dir: impl Into<String>,
    ) -> Self {
        Self {
            role: PeerLifecycleRole::LocalDaemon,
            target: target.into(),
            platform: local_platform(),
            scope: if provider.requires_elevation() {
                PeerLifecycleScope::System
            } else {
                PeerLifecycleScope::User
            },
            provider,
            service_name: service_name.into(),
            binary_path: binary_path.into(),
            transport,
            control_endpoint,
            token,
            state_dir: state_dir.into(),
            rollback_policy: RollbackPolicy::StopAndRestore,
        }
    }
}

fn platform_from_remote_install(
    platform: RemotePlatform,
    persistence: PersistenceMode,
) -> PeerLifecyclePlatform {
    match persistence {
        PersistenceMode::Launchd => PeerLifecyclePlatform::Macos,
        PersistenceMode::Schtasks => PeerLifecyclePlatform::Windows,
        _ => match platform {
            RemotePlatform::Windows => PeerLifecyclePlatform::Windows,
            RemotePlatform::Unix => PeerLifecyclePlatform::Linux,
            RemotePlatform::Auto => PeerLifecyclePlatform::Unknown,
        },
    }
}

fn scope_from_persistence(persistence: PersistenceMode) -> PeerLifecycleScope {
    match persistence {
        PersistenceMode::None => PeerLifecycleScope::None,
        PersistenceMode::Nohup => PeerLifecycleScope::Managed,
        PersistenceMode::Systemd | PersistenceMode::Launchd | PersistenceMode::Schtasks => {
            PeerLifecycleScope::User
        }
        PersistenceMode::Auto => PeerLifecycleScope::User,
    }
}

fn remote_state_dir(platform: RemotePlatform) -> String {
    match platform {
        RemotePlatform::Windows => "%USERPROFILE%\\.ssh_proxy".to_string(),
        RemotePlatform::Unix | RemotePlatform::Auto => "$HOME/.ssh_proxy".to_string(),
    }
}

fn local_platform() -> PeerLifecyclePlatform {
    if cfg!(windows) {
        PeerLifecyclePlatform::Windows
    } else if cfg!(target_os = "macos") {
        PeerLifecyclePlatform::Macos
    } else if cfg!(target_os = "linux") {
        PeerLifecyclePlatform::Linux
    } else {
        PeerLifecyclePlatform::Unknown
    }
}

#[cfg(test)]
mod tests {
    use ssh_proxy_core::{intent::SshTargetIntent, model::PersistenceMode};

    use super::*;

    #[test]
    fn remote_peer_spec_uses_command_neutral_intent() {
        let intent = RemoteInstallIntent::new(
            SshTargetIntent::new("edge"),
            "127.0.0.1:19080".parse().unwrap(),
            "127.0.0.1:19081".parse().unwrap(),
            PersistenceMode::Systemd,
        );
        let spec = PeerLifecycleSpec::remote_peer_from_intent(
            "edge",
            "/home/me/bin/ssh_proxy",
            &intent,
            ServiceProviderKind::SystemdUser,
        );

        assert_eq!(spec.role, PeerLifecycleRole::RemotePeer);
        assert_eq!(spec.scope, PeerLifecycleScope::User);
        assert_eq!(spec.provider.manager_name(), "systemd_user");
        assert_eq!(
            spec.control_endpoint.as_deref(),
            Some("tcp://127.0.0.1:19081")
        );
    }
}
