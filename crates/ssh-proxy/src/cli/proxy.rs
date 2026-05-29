use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;
use serde::{Deserialize, Serialize};

use crate::peer_transport;

use super::{DeployMode, RemoteOs, RemoteTransport, RouteWorkloadHint, TcpTarget};

#[derive(Debug, Clone, Parser, Serialize, Deserialize)]
pub struct ProxyArgs {
    #[arg(help = "SSH target, e.g. host, user@host, or an alias from ~/.ssh/config")]
    pub target: String,

    #[arg(short, long, default_value = "127.0.0.1:1080")]
    pub listen: SocketAddr,

    #[arg(
        long,
        value_name = "HOST:PORT",
        help = "Expose a raw TCP tunnel to a fixed egress target instead of SOCKS/HTTP proxy mode"
    )]
    pub tcp_target: Option<TcpTarget>,

    #[arg(
        long = "ssh-arg",
        help = "Extra argument passed to ssh before TARGET. Repeat for -F, -i, -p, etc."
    )]
    #[serde(default)]
    pub ssh_args: Vec<String>,

    #[arg(
        long,
        help = "Rejected when using russh; kept to produce a clear migration error"
    )]
    pub ssh_command: Option<String>,

    #[arg(long, help = "Override SSH username")]
    pub user: Option<String>,

    #[arg(short = 'p', long, help = "Override SSH port")]
    pub port: Option<u16>,

    #[arg(short = 'i', long, help = "Identity file. Repeatable")]
    #[serde(default)]
    pub identity: Vec<PathBuf>,

    #[arg(short = 'F', long, help = "OpenSSH config path")]
    pub config: Option<PathBuf>,

    #[arg(long, help = "known_hosts path")]
    pub known_hosts: Option<PathBuf>,

    #[arg(long, help = "Learn unknown host keys into known_hosts")]
    pub accept_new: bool,

    #[arg(
        long,
        alias = "no-known-hosts",
        help = "Disable SSH host-key verification for this target. Insecure; use only for disposable bootstrap."
    )]
    #[serde(default)]
    pub insecure_ignore_host_key: bool,

    #[arg(
        short = 'J',
        long = "jump",
        help = "ProxyJump hop. Supports comma-separated chains and user@host:port"
    )]
    #[serde(default)]
    pub jump: Vec<String>,

    #[arg(
        long,
        help = "Already installed remote executable path. If absent, self upload is used unless --deploy never"
    )]
    pub remote_path: Option<String>,

    #[arg(
        long,
        help = "Local executable to upload instead of the currently running executable"
    )]
    pub remote_bin: Option<PathBuf>,

    #[arg(long, default_value = "auto")]
    pub deploy: DeployMode,

    #[arg(long, default_value = "auto")]
    pub remote_os: RemoteOs,

    #[arg(long, default_value = "auto")]
    pub remote_transport: RemoteTransport,

    #[arg(long, default_value = "127.0.0.1:19080")]
    pub remote_tcp: SocketAddr,

    #[arg(long, default_value = "127.0.0.1:19081")]
    pub remote_control: SocketAddr,

    #[arg(long, help = "Direct QUIC peer transport address")]
    pub remote_quic: Option<SocketAddr>,

    #[arg(long, help = "Allow insecure direct TCP peer transport in auto mode")]
    #[serde(default)]
    pub allow_plain_tcp: bool,

    #[arg(long, help = "Direct TLS peer transport address")]
    pub remote_tls: Option<SocketAddr>,

    #[arg(long, help = "CA/root certificate PEM for direct TLS peer transport")]
    pub remote_ca: Option<PathBuf>,

    #[arg(long, default_value = "localhost")]
    pub remote_name: String,

    #[arg(long, help = "PEM client certificate chain for mTLS peer transport")]
    pub remote_client_cert: Option<PathBuf>,

    #[arg(long, help = "PEM client private key for mTLS peer transport")]
    pub remote_client_key: Option<PathBuf>,

    #[arg(long)]
    pub remote_token: Option<String>,

    #[arg(
        long,
        help = "Optional upstream proxy used by the egress side, e.g. http://127.0.0.1:<proxy-port> or socks5h://127.0.0.1:<proxy-port>"
    )]
    pub egress_proxy: Option<String>,

    #[arg(long, default_value_t = 5)]
    pub reconnect_delay_secs: u64,

    #[arg(long, default_value_t = 60)]
    pub reconnect_max_delay_secs: u64,

    #[arg(long, default_value_t = 30)]
    pub connect_timeout_secs: u64,

    #[arg(
        long,
        default_value_t = 1,
        help = "Number of peer transport connections kept for this route"
    )]
    pub transport_pool_size: usize,

    #[arg(skip)]
    #[serde(default)]
    pub pool_policy: Option<String>,

    #[arg(skip)]
    #[serde(default)]
    pub workload_hint: Option<RouteWorkloadHint>,

    #[arg(long, default_value_t = peer_transport::QUIC_MAX_BIDI_STREAMS)]
    #[serde(default = "peer_transport::default_quic_max_bidi_streams")]
    pub quic_max_bidi_streams: u32,

    #[arg(long, default_value_t = peer_transport::QUIC_STREAM_RECEIVE_WINDOW)]
    #[serde(default = "peer_transport::default_quic_stream_receive_window")]
    pub quic_stream_receive_window: u32,

    #[arg(long, default_value_t = peer_transport::QUIC_RECEIVE_WINDOW)]
    #[serde(default = "peer_transport::default_quic_receive_window")]
    pub quic_receive_window: u32,

    #[arg(long, default_value_t = peer_transport::QUIC_KEEP_ALIVE_INTERVAL_SECS)]
    #[serde(default = "peer_transport::default_quic_keep_alive_interval_secs")]
    pub quic_keep_alive_interval_secs: u64,

    #[arg(long, default_value_t = peer_transport::QUIC_IDLE_TIMEOUT_SECS)]
    #[serde(default = "peer_transport::default_quic_idle_timeout_secs")]
    pub quic_idle_timeout_secs: u64,

    #[arg(
        long,
        help = "Number of SSH sessions kept for ssh-native direct-tcpip routes"
    )]
    #[serde(default)]
    pub ssh_session_pool_size: Option<usize>,

    #[arg(skip)]
    #[serde(default)]
    pub ssh_session_pool_source: Option<String>,

    #[arg(skip)]
    #[serde(default)]
    pub ssh_session_pool_reason: Option<String>,

    #[arg(skip)]
    #[serde(default)]
    pub ssh_session_pool_warning: Option<String>,

    #[arg(skip)]
    #[serde(default)]
    pub transport_pool_source: Option<String>,

    #[arg(skip)]
    #[serde(default)]
    pub transport_pool_reason: Option<String>,

    #[arg(skip)]
    #[serde(default)]
    pub transport_selection_source: Option<String>,

    #[arg(skip)]
    #[serde(default)]
    pub transport_selection_reason: Option<String>,

    #[arg(skip)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preflight_recommended_fallback: Option<String>,

    #[arg(skip)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preflight_selected_reason: Option<String>,

    #[arg(skip)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preflight_repair_hint: Option<String>,

    #[arg(skip)]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preflight_candidate_failures: Vec<serde_json::Value>,

    #[arg(long)]
    pub no_reconnect: bool,

    #[arg(long, help = "Local TCP control API, e.g. 127.0.0.1:1081")]
    pub control_listen: Option<SocketAddr>,
}
