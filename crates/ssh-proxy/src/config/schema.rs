use std::{collections::HashMap, net::SocketAddr, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::cli;

pub const CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_config_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub identity: NodeIdentity,
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub defaults: ProxyProfile,
    #[serde(default)]
    pub profiles: HashMap<String, ProxyProfile>,
    #[serde(default)]
    pub peers: HashMap<String, PeerRecord>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            identity: NodeIdentity::default(),
            daemon: DaemonConfig::default(),
            defaults: ProxyProfile::default(),
            profiles: HashMap::new(),
            peers: HashMap::new(),
        }
    }
}

fn default_config_schema_version() -> u32 {
    CONFIG_SCHEMA_VERSION
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeIdentity {
    pub node_id: Option<String>,
    pub node_name: Option<String>,
    pub secret: Option<String>,
    pub cert: Option<PathBuf>,
    pub key: Option<PathBuf>,
    pub ca: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub control_listen: Option<SocketAddr>,
    pub control_endpoint: Option<String>,
    pub transport_listen: Option<SocketAddr>,
    pub tls_transport_listen: Option<SocketAddr>,
    pub quic_transport_listen: Option<SocketAddr>,
    pub quic_max_bidi_streams: Option<u32>,
    pub quic_stream_receive_window: Option<u32>,
    pub quic_receive_window: Option<u32>,
    pub quic_keep_alive_interval_secs: Option<u64>,
    pub quic_idle_timeout_secs: Option<u64>,
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
    pub tls_client_ca: Option<PathBuf>,
    pub token: Option<String>,
    pub token_metadata: Option<TokenMetadata>,
    #[serde(default)]
    pub report_to: Vec<String>,
    pub routes_path: Option<PathBuf>,
    pub route_autostart: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMetadata {
    pub created_at_unix: u64,
    pub rotated_at_unix: Option<u64>,
    pub scope: String,
    pub expires_at_unix: Option<u64>,
    #[serde(default = "default_token_generation")]
    pub generation: u64,
}

impl TokenMetadata {
    pub fn new(scope: impl Into<String>) -> Self {
        Self {
            created_at_unix: super::now_unix(),
            rotated_at_unix: None,
            scope: scope.into(),
            expires_at_unix: None,
            generation: 1,
        }
    }

    pub fn rotated(scope: impl Into<String>, generation: u64) -> Self {
        let now = super::now_unix();
        Self {
            created_at_unix: now,
            rotated_at_unix: Some(now),
            scope: scope.into(),
            expires_at_unix: None,
            generation: generation.max(1),
        }
    }
}

fn default_token_generation() -> u64 {
    1
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProxyProfile {
    pub target: Option<String>,
    pub listen: Option<SocketAddr>,
    pub tcp_target: Option<cli::TcpTarget>,
    #[serde(default)]
    pub ssh_args: Vec<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    #[serde(default)]
    pub identity: Vec<PathBuf>,
    pub config: Option<PathBuf>,
    pub known_hosts: Option<PathBuf>,
    pub accept_new: Option<bool>,
    pub insecure_ignore_host_key: Option<bool>,
    #[serde(default)]
    pub jump: Vec<String>,
    pub remote_path: Option<String>,
    pub remote_bin: Option<PathBuf>,
    pub deploy: Option<String>,
    pub remote_os: Option<String>,
    pub remote_transport: Option<String>,
    pub remote_tcp: Option<SocketAddr>,
    pub remote_control: Option<SocketAddr>,
    pub remote_quic: Option<SocketAddr>,
    pub allow_plain_tcp: Option<bool>,
    pub remote_tls: Option<SocketAddr>,
    pub quic_max_bidi_streams: Option<u32>,
    pub quic_stream_receive_window: Option<u32>,
    pub quic_receive_window: Option<u32>,
    pub quic_keep_alive_interval_secs: Option<u64>,
    pub quic_idle_timeout_secs: Option<u64>,
    pub remote_ca: Option<PathBuf>,
    pub remote_name: Option<String>,
    pub remote_client_cert: Option<PathBuf>,
    pub remote_client_key: Option<PathBuf>,
    pub remote_token: Option<String>,
    pub egress_proxy: Option<String>,
    pub reconnect_delay_secs: Option<u64>,
    pub reconnect_max_delay_secs: Option<u64>,
    pub connect_timeout_secs: Option<u64>,
    pub transport_pool_size: Option<usize>,
    pub workload_hint: Option<cli::RouteWorkloadHint>,
    pub ssh_session_pool_size: Option<usize>,
    pub no_reconnect: Option<bool>,
    pub control_listen: Option<SocketAddr>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PeerRecord {
    pub node_id: Option<String>,
    pub node_name: Option<String>,
    pub service_instance_id: Option<String>,
    pub version: Option<String>,
    pub control_api_version: Option<u16>,
    pub peer_protocol_version: Option<u16>,
    #[serde(default)]
    pub features: Vec<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub os_user: Option<String>,
    pub data_dir: Option<PathBuf>,
    pub target: Option<String>,
    pub trust: Option<String>,
    pub remote_path: Option<String>,
    pub control_endpoint: Option<String>,
    pub transport: Option<SocketAddr>,
    pub tls_transport: Option<SocketAddr>,
    pub quic_transport: Option<SocketAddr>,
    #[serde(default)]
    pub transport_protocols: Vec<String>,
    pub token: Option<String>,
    pub token_metadata: Option<TokenMetadata>,
    pub tls_server_cert_fingerprint: Option<String>,
    pub tls_client_ca_fingerprint: Option<String>,
    pub last_seen_unix: Option<u64>,
}

impl PeerRecord {
    pub fn known_transport_protocols(&self) -> Vec<String> {
        if !self.transport_protocols.is_empty() {
            return self.transport_protocols.clone();
        }
        let mut protocols = Vec::new();
        if self.quic_transport.is_some() {
            protocols.push("quic".to_string());
        }
        if self.tls_transport.is_some() {
            protocols.push("tls-tcp".to_string());
        }
        if self.transport.is_some() {
            protocols.push("plain-tcp".to_string());
        }
        protocols
    }
}
