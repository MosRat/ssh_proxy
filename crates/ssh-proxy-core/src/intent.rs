use std::{fmt, net::SocketAddr, path::PathBuf, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::model::{
    PersistenceMode, RemotePlatform, RouteConnectMode, RouteDirection, TcpTarget, TransportMode,
    WorkloadHint,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeploymentPolicy {
    #[default]
    #[serde(alias = "Auto")]
    Auto,
    #[serde(alias = "Always")]
    Always,
    #[serde(alias = "Never")]
    Never,
}

impl fmt::Display for DeploymentPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        })
    }
}

impl FromStr for DeploymentPolicy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match normalize(value).as_str() {
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            "never" => Ok(Self::Never),
            _ => Err(format!("unknown deployment policy {value:?}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshTargetIntent {
    pub target: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ssh_args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identity: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_hosts: Option<PathBuf>,
    #[serde(default)]
    pub accept_new: bool,
    #[serde(default)]
    pub insecure_ignore_host_key: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jump: Vec<String>,
}

impl SshTargetIntent {
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
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
        }
    }

    pub fn requires_external_ssh(&self) -> bool {
        self.ssh_command.is_some()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuicRuntimeTuningIntent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bidi_streams: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_receive_window: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receive_window: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_alive_interval_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout_secs: Option<u64>,
}

impl QuicRuntimeTuningIntent {
    pub fn is_empty(&self) -> bool {
        self.max_bidi_streams.is_none()
            && self.stream_receive_window.is_none()
            && self.receive_window.is_none()
            && self.keep_alive_interval_secs.is_none()
            && self.idle_timeout_secs.is_none()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTuningIntent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconnect_delay_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconnect_max_delay_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_pool_size: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_session_pool_size: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload_hint: Option<WorkloadHint>,
    #[serde(default, skip_serializing_if = "QuicRuntimeTuningIntent::is_empty")]
    pub quic: QuicRuntimeTuningIntent,
    #[serde(default)]
    pub no_reconnect: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteEndpointIntent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen: Option<SocketAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_listen: Option<SocketAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_listen: Option<SocketAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_peer: Option<SocketAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tcp_target: Option<TcpTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_tcp: Option<SocketAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_control: Option<SocketAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_quic: Option<SocketAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_tls: Option<SocketAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_ca: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_client_cert: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_client_key: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress_proxy: Option<String>,
    #[serde(default)]
    pub allow_plain_tcp: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteIntent {
    pub ssh: SshTargetIntent,
    pub direction: RouteDirection,
    pub connect_mode: RouteConnectMode,
    #[serde(default)]
    pub transport: TransportMode,
    #[serde(default)]
    pub remote_platform: RemotePlatform,
    #[serde(default)]
    pub deployment: DeploymentPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_bin: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_token: Option<String>,
    #[serde(default)]
    pub endpoint: RouteEndpointIntent,
    #[serde(default)]
    pub runtime: RuntimeTuningIntent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default)]
    pub persist: bool,
}

impl RouteIntent {
    pub fn new(ssh: SshTargetIntent, direction: RouteDirection) -> Self {
        Self {
            ssh,
            direction,
            connect_mode: RouteConnectMode::Auto,
            transport: TransportMode::Auto,
            remote_platform: RemotePlatform::Auto,
            deployment: DeploymentPolicy::Auto,
            remote_path: None,
            remote_bin: None,
            remote_token: None,
            endpoint: RouteEndpointIntent::default(),
            runtime: RuntimeTuningIntent::default(),
            id: None,
            persist: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteInstallIntent {
    pub ssh: SshTargetIntent,
    #[serde(default)]
    pub remote_platform: RemotePlatform,
    pub persistence: PersistenceMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_bin: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_token: Option<String>,
    pub remote_tcp: SocketAddr,
    pub remote_control: SocketAddr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_tls_transport: Option<SocketAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_quic_transport: Option<SocketAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_tls_cert: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_tls_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_tls_client_ca: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_node_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_control_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_transport: Option<SocketAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_node_name: Option<String>,
}

impl RemoteInstallIntent {
    pub fn new(
        ssh: SshTargetIntent,
        remote_tcp: SocketAddr,
        remote_control: SocketAddr,
        persistence: PersistenceMode,
    ) -> Self {
        Self {
            ssh,
            remote_platform: RemotePlatform::Auto,
            persistence,
            remote_path: None,
            remote_bin: None,
            remote_token: None,
            remote_tcp,
            remote_control,
            remote_tls_transport: None,
            remote_quic_transport: None,
            remote_tls_cert: None,
            remote_tls_key: None,
            remote_tls_client_ca: None,
            local_node_id: None,
            local_node_name: None,
            local_control_endpoint: None,
            local_transport: None,
            remote_node_id: None,
            remote_node_name: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerBootstrapIntent {
    pub install: RemoteInstallIntent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default)]
    pub force: bool,
}

impl PeerBootstrapIntent {
    pub fn new(install: RemoteInstallIntent) -> Self {
        Self {
            install,
            alias: None,
            force: false,
        }
    }
}

fn normalize(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    for (idx, ch) in value.chars().enumerate() {
        if ch == '_' {
            normalized.push('-');
        } else if ch.is_ascii_uppercase() {
            if idx > 0 {
                normalized.push('-');
            }
            normalized.push(ch.to_ascii_lowercase());
        } else {
            normalized.push(ch.to_ascii_lowercase());
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn ssh_intent_marks_legacy_external_command() {
        let mut intent = SshTargetIntent::new("user@example.com");

        assert!(!intent.requires_external_ssh());

        intent.ssh_command = Some("ssh".to_string());

        assert!(intent.requires_external_ssh());
    }

    #[test]
    fn route_intent_serializes_command_neutral_shape() {
        let mut intent = RouteIntent::new(
            SshTargetIntent::new("edge"),
            RouteDirection::RemoteUsesLocal,
        );
        intent.endpoint.remote_listen = Some("127.0.0.1:18080".parse().unwrap());
        intent.endpoint.control_listen = Some("127.0.0.1:18081".parse().unwrap());
        intent.endpoint.tcp_target = Some(TcpTarget {
            host: "db.internal".to_string(),
            port: 5432,
        });
        intent.transport = TransportMode::SshNative;
        intent.runtime.transport_pool_size = Some(4);
        intent.persist = false;

        let value = serde_json::to_value(intent).unwrap();

        assert_eq!(value["direction"], "remote-uses-local");
        assert_eq!(value["transport"], "ssh-native");
        assert_eq!(value["endpoint"]["remote_listen"], "127.0.0.1:18080");
        assert_eq!(value["endpoint"]["control_listen"], "127.0.0.1:18081");
        assert_eq!(value["endpoint"]["tcp_target"]["host"], "db.internal");
        assert_eq!(value["runtime"]["transport_pool_size"], 4);
        assert_eq!(value["persist"], false);
        assert!(value["ssh"].get("ssh_args").is_none());
    }

    #[test]
    fn remote_install_intent_keeps_service_endpoints_typed() {
        let install = RemoteInstallIntent::new(
            SshTargetIntent::new("prod"),
            "127.0.0.1:19080".parse().unwrap(),
            "127.0.0.1:19081".parse().unwrap(),
            PersistenceMode::Systemd,
        );
        let value = serde_json::to_value(install).unwrap();

        assert_eq!(value["remote_tcp"], "127.0.0.1:19080");
        assert_eq!(value["remote_control"], "127.0.0.1:19081");
        assert_eq!(value["persistence"], "systemd");
        assert_eq!(value["remote_platform"], "auto");
    }

    #[test]
    fn peer_bootstrap_intent_is_additive_over_install() {
        let install = RemoteInstallIntent::new(
            SshTargetIntent::new("box"),
            "127.0.0.1:19080".parse().unwrap(),
            "127.0.0.1:19081".parse().unwrap(),
            PersistenceMode::None,
        );
        let mut intent = PeerBootstrapIntent::new(install);
        intent.alias = Some("lab".to_string());
        intent.force = true;

        assert_eq!(
            serde_json::to_value(intent).unwrap(),
            json!({
                "install": {
                    "ssh": {
                        "target": "box",
                        "accept_new": false,
                        "insecure_ignore_host_key": false
                    },
                    "remote_platform": "auto",
                    "persistence": "none",
                    "remote_tcp": "127.0.0.1:19080",
                    "remote_control": "127.0.0.1:19081"
                },
                "alias": "lab",
                "force": true
            })
        );
    }

    #[test]
    fn deployment_policy_accepts_legacy_aliases() {
        assert_eq!("Auto".parse(), Ok(DeploymentPolicy::Auto));
        assert_eq!("always".parse(), Ok(DeploymentPolicy::Always));
        assert_eq!(
            serde_json::to_string(&DeploymentPolicy::Never).unwrap(),
            "\"never\""
        );
    }
}
