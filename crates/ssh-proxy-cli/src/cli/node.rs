use std::{net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

use ssh_proxy_control as control_socket;
use ssh_proxy_transport::peer_transport;

use super::{DeployMode, RemoteOs, RemoteTransport, RouteArgs, RouteWorkloadHint, TcpTarget};

#[derive(Debug, Clone, Parser)]
pub struct NodeArgs {
    #[command(subcommand)]
    pub command: NodeCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum NodeCommand {
    Daemon(NodeDaemonArgs),
    Control(NodeControlArgs),
}

#[derive(Debug, Clone, Parser)]
pub struct NodeDaemonArgs {
    #[arg(long, default_value_t = control_socket::default_endpoint_string())]
    pub control: String,

    #[arg(long, help = "Framed transport listener for proxy peers")]
    pub transport: Option<SocketAddr>,

    #[arg(long, help = "Shared token required by framed transport clients")]
    pub token: Option<String>,

    #[arg(long, help = "Direct TLS framed transport listener for proxy peers")]
    pub tls_transport: Option<SocketAddr>,

    #[arg(long, help = "Direct QUIC framed transport listener for proxy peers")]
    pub quic_transport: Option<SocketAddr>,

    #[arg(long)]
    pub quic_max_bidi_streams: Option<u32>,

    #[arg(long)]
    pub quic_stream_receive_window: Option<u32>,

    #[arg(long)]
    pub quic_receive_window: Option<u32>,

    #[arg(long)]
    pub quic_keep_alive_interval_secs: Option<u64>,

    #[arg(long)]
    pub quic_idle_timeout_secs: Option<u64>,

    #[arg(long, help = "PEM certificate chain for --tls-transport")]
    pub tls_cert: Option<PathBuf>,

    #[arg(long, help = "PEM private key for --tls-transport")]
    pub tls_key: Option<PathBuf>,

    #[arg(long, help = "PEM CA/root used to verify TLS peer client certificates")]
    pub tls_client_ca: Option<PathBuf>,

    #[arg(long, help = "Stable node name shown in status and reports")]
    pub name: Option<String>,

    #[arg(
        long = "report-to",
        help = "Peer node control endpoint to receive status reports"
    )]
    pub report_to: Vec<String>,

    #[arg(long, default_value_t = 30)]
    pub report_interval_secs: u64,

    #[arg(long, help = "Path to daemon-owned persistent route state")]
    pub routes_path: Option<PathBuf>,

    #[arg(long, help = "Do not restore persistent routes on daemon startup")]
    pub no_route_autostart: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct NodeControlArgs {
    #[arg(long, default_value_t = control_socket::default_endpoint_string())]
    pub endpoint: String,

    #[arg(
        long,
        help = "Token for TCP node control endpoints; defaults to [daemon].token"
    )]
    pub token: Option<String>,

    #[arg(long, help = "Emit machine-readable JSON output")]
    pub json: bool,

    #[command(subcommand)]
    pub command: NodeControlCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum NodeControlCommand {
    Status,
    Descriptor,
    Shutdown,
    Nodes,
    Jobs,
    NodeEnsure {
        #[arg(long, value_enum, default_value = "user")]
        scope: NodeControlScope,
    },
    NodeStart {
        id: String,
    },
    NodeStop {
        id: String,
    },
    NodeRestart {
        id: String,
    },
    Connect {
        profile: String,
    },
    Disconnect {
        profile: String,
    },
    Forward(NodeForwardArgs),
    Reverse(NodeReverseArgs),
    RoutePlan(RouteArgs),
    StopRoute {
        id: String,
    },
    RestartRoute {
        id: String,
    },
    Routes,
    Peers,
    TokenRotate,
    PeerBootstrap(PeerBootstrapArgs),
    PeerEnsure(PeerBootstrapArgs),
    PeerUpdate(PeerBootstrapArgs),
    PeerRefresh(PeerBootstrapArgs),
    PeerDiff(PeerBootstrapArgs),
    PeerReconcile(PeerBootstrapArgs),
    PeerCheckVersion(PeerBootstrapArgs),
    PeerRotateToken(PeerBootstrapArgs),
    PeerForget {
        alias: String,
    },
    Links,
    #[command(hide = true)]
    Send {
        json: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeControlScope {
    User,
    System,
    Session,
}

impl NodeControlScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::System => "system",
            Self::Session => "session",
        }
    }
}

#[derive(Debug, Clone, Parser)]
pub struct NodeForwardArgs {
    #[arg(help = "SSH target, e.g. host, user@host, or alias")]
    pub target: String,

    #[arg(short, long, default_value = "127.0.0.1:1080")]
    pub listen: SocketAddr,

    #[arg(
        long,
        value_name = "HOST:PORT",
        help = "Expose a raw TCP tunnel to this fixed egress target instead of SOCKS/HTTP proxy mode"
    )]
    pub tcp_target: Option<TcpTarget>,

    #[arg(long = "ssh-arg")]
    pub ssh_args: Vec<String>,

    #[arg(long)]
    pub user: Option<String>,

    #[arg(short = 'p', long)]
    pub port: Option<u16>,

    #[arg(short = 'i', long)]
    pub identity: Vec<PathBuf>,

    #[arg(short = 'F', long)]
    pub config: Option<PathBuf>,

    #[arg(long)]
    pub known_hosts: Option<PathBuf>,

    #[arg(long)]
    pub accept_new: bool,

    #[arg(long, alias = "no-known-hosts")]
    pub insecure_ignore_host_key: bool,

    #[arg(short = 'J', long = "jump")]
    pub jump: Vec<String>,

    #[arg(long)]
    pub remote_path: Option<String>,

    #[arg(long)]
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
    pub pool_policy: Option<String>,

    #[arg(skip)]
    pub workload_hint: Option<RouteWorkloadHint>,

    #[arg(long, default_value_t = peer_transport::QUIC_MAX_BIDI_STREAMS)]
    pub quic_max_bidi_streams: u32,

    #[arg(long, default_value_t = peer_transport::QUIC_STREAM_RECEIVE_WINDOW)]
    pub quic_stream_receive_window: u32,

    #[arg(long, default_value_t = peer_transport::QUIC_RECEIVE_WINDOW)]
    pub quic_receive_window: u32,

    #[arg(long, default_value_t = peer_transport::QUIC_KEEP_ALIVE_INTERVAL_SECS)]
    pub quic_keep_alive_interval_secs: u64,

    #[arg(long, default_value_t = peer_transport::QUIC_IDLE_TIMEOUT_SECS)]
    pub quic_idle_timeout_secs: u64,

    #[arg(
        long,
        help = "Number of SSH sessions kept for ssh-native direct-tcpip routes"
    )]
    pub ssh_session_pool_size: Option<usize>,

    #[arg(skip)]
    pub ssh_session_pool_source: Option<String>,

    #[arg(skip)]
    pub ssh_session_pool_reason: Option<String>,

    #[arg(skip)]
    pub ssh_session_pool_warning: Option<String>,

    #[arg(skip)]
    pub transport_pool_source: Option<String>,

    #[arg(skip)]
    pub transport_pool_reason: Option<String>,

    #[arg(skip)]
    pub transport_selection_source: Option<String>,

    #[arg(skip)]
    pub transport_selection_reason: Option<String>,

    #[arg(skip)]
    pub preflight_recommended_fallback: Option<String>,

    #[arg(skip)]
    pub preflight_selected_reason: Option<String>,

    #[arg(skip)]
    pub preflight_repair_hint: Option<String>,

    #[arg(skip)]
    pub preflight_candidate_failures: Vec<serde_json::Value>,

    #[arg(long)]
    pub no_reconnect: bool,

    #[arg(long)]
    pub id: Option<String>,

    #[arg(long, help = "Do not save this route for daemon restart recovery")]
    pub volatile: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct NodeReverseArgs {
    #[arg(help = "SSH target, e.g. host, user@host, or alias")]
    pub target: String,

    #[arg(long, default_value = "127.0.0.1:1080")]
    pub remote_listen: SocketAddr,

    #[arg(
        long,
        value_name = "HOST:PORT",
        help = "Expose a raw TCP tunnel to this fixed local egress target instead of SOCKS/HTTP proxy mode"
    )]
    pub tcp_target: Option<TcpTarget>,

    #[arg(long = "ssh-arg")]
    pub ssh_args: Vec<String>,

    #[arg(long)]
    pub user: Option<String>,

    #[arg(short = 'p', long)]
    pub port: Option<u16>,

    #[arg(short = 'i', long)]
    pub identity: Vec<PathBuf>,

    #[arg(short = 'F', long)]
    pub config: Option<PathBuf>,

    #[arg(long)]
    pub known_hosts: Option<PathBuf>,

    #[arg(long)]
    pub accept_new: bool,

    #[arg(long, alias = "no-known-hosts")]
    pub insecure_ignore_host_key: bool,

    #[arg(short = 'J', long = "jump")]
    pub jump: Vec<String>,

    #[arg(long)]
    pub remote_path: Option<String>,

    #[arg(long)]
    pub remote_bin: Option<PathBuf>,

    #[arg(long, default_value = "auto")]
    pub deploy: DeployMode,

    #[arg(long, default_value = "auto")]
    pub remote_os: RemoteOs,

    #[arg(
        long,
        help = "Optional upstream proxy used by this local egress side in reverse-link routes"
    )]
    pub egress_proxy: Option<String>,

    #[arg(long, default_value_t = 5)]
    pub reconnect_delay_secs: u64,

    #[arg(long, default_value_t = 60)]
    pub reconnect_max_delay_secs: u64,

    #[arg(long, default_value_t = 30)]
    pub connect_timeout_secs: u64,

    #[arg(skip)]
    pub transport_pool_source: Option<String>,

    #[arg(skip)]
    pub transport_pool_reason: Option<String>,

    #[arg(long)]
    pub no_reconnect: bool,

    #[arg(long)]
    pub id: Option<String>,

    #[arg(long, help = "Do not save this route for daemon restart recovery")]
    pub volatile: bool,
}

#[derive(Debug, Clone, Parser, Serialize, Deserialize)]
pub struct PeerBootstrapArgs {
    #[arg(help = "SSH target, e.g. host, user@host, or alias")]
    pub target: String,

    #[arg(long, help = "Record this peer under a different local alias")]
    pub alias: Option<String>,

    #[arg(long, help = "Refresh even when a usable peer record already exists")]
    pub force: bool,

    #[arg(long = "ssh-arg")]
    pub ssh_args: Vec<String>,

    #[arg(long)]
    pub user: Option<String>,

    #[arg(short = 'p', long)]
    pub port: Option<u16>,

    #[arg(short = 'i', long)]
    pub identity: Vec<PathBuf>,

    #[arg(short = 'F', long)]
    pub config: Option<PathBuf>,

    #[arg(long)]
    pub known_hosts: Option<PathBuf>,

    #[arg(long)]
    pub accept_new: bool,

    #[arg(long, alias = "no-known-hosts")]
    pub insecure_ignore_host_key: bool,

    #[arg(short = 'J', long = "jump")]
    pub jump: Vec<String>,

    #[arg(long)]
    pub remote_path: Option<String>,

    #[arg(long)]
    pub remote_bin: Option<PathBuf>,

    #[arg(long, default_value = "auto")]
    pub remote_os: RemoteOs,

    #[arg(long)]
    pub remote_token: Option<String>,

    #[arg(long, default_value = "127.0.0.1:19080")]
    pub remote_tcp: SocketAddr,

    #[arg(long, default_value = "127.0.0.1:19081")]
    pub remote_control: SocketAddr,
}
