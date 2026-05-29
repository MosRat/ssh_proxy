use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
};

use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use ssh_proxy_core::model;

use ssh_proxy_control as control_socket;

use super::{DeployMode, RemoteOs, RemoteTransport, TcpTarget};

#[derive(Debug, Clone, Parser, Serialize, Deserialize)]
pub struct RouteArgs {
    #[arg(help = "SSH target, e.g. host, user@host, or an alias from ~/.ssh/config")]
    pub target: String,

    #[arg(long, value_enum, default_value = "local-uses-remote")]
    pub direction: RouteDirection,

    #[arg(
        long,
        value_enum,
        default_value = "auto",
        help = "Connection plan for remote-uses-local: direct peer transport or local-initiated reverse link"
    )]
    pub connect_mode: RouteConnectMode,

    #[arg(
        long,
        default_value_t = 18080,
        help = "Proxy listener port on the side selected by --direction"
    )]
    pub port: u16,

    #[arg(long, default_value = "127.0.0.1", help = "Proxy listener bind IP")]
    pub bind: IpAddr,

    #[arg(
        long,
        value_name = "HOST:PORT",
        help = "Expose a raw TCP tunnel to this fixed egress target instead of SOCKS/HTTP proxy mode"
    )]
    pub tcp_target: Option<TcpTarget>,

    #[arg(long, default_value_t = control_socket::default_endpoint_string())]
    pub endpoint: String,

    #[arg(
        long,
        help = "Token for TCP node control endpoints; defaults to [daemon].token"
    )]
    pub token: Option<String>,

    #[arg(long = "ssh-arg")]
    pub ssh_args: Vec<String>,

    #[arg(long)]
    pub user: Option<String>,

    #[arg(short = 'p', long)]
    pub ssh_port: Option<u16>,

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

    #[arg(long)]
    pub remote_tcp: Option<SocketAddr>,

    #[arg(long)]
    pub remote_control: Option<SocketAddr>,

    #[arg(long)]
    pub remote_quic: Option<SocketAddr>,

    #[arg(long)]
    pub remote_tls: Option<SocketAddr>,

    #[arg(long)]
    pub remote_ca: Option<PathBuf>,

    #[arg(long, default_value = "localhost")]
    pub remote_name: String,

    #[arg(long)]
    pub remote_token: Option<String>,

    #[arg(
        long,
        help = "Optional upstream proxy used by the egress side, e.g. http://127.0.0.1:<proxy-port> or socks5h://127.0.0.1:<proxy-port>"
    )]
    pub egress_proxy: Option<String>,

    #[arg(long, help = "Initial reconnect delay for daemon-owned route tasks")]
    pub reconnect_delay_secs: Option<u64>,

    #[arg(long, help = "Maximum reconnect delay for daemon-owned route tasks")]
    pub reconnect_max_delay_secs: Option<u64>,

    #[arg(long, help = "Connection timeout for bootstrap and peer transports")]
    pub connect_timeout_secs: Option<u64>,

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

    #[arg(
        long,
        help = "Number of peer transport connections kept for this route"
    )]
    pub transport_pool_size: Option<usize>,

    #[arg(
        long,
        value_enum,
        help = "Workload hint for automatic transport pool sizing: large, concurrent, or mixed"
    )]
    pub workload_hint: Option<RouteWorkloadHint>,

    #[arg(
        long,
        help = "Number of SSH sessions kept for ssh-native direct-tcpip routes"
    )]
    #[serde(default)]
    pub ssh_session_pool_size: Option<usize>,

    #[arg(
        long,
        help = "Do not reconnect this route after the first bridge exits"
    )]
    #[serde(default)]
    pub no_reconnect: bool,

    #[arg(
        long,
        help = "Address the remote node can use to reach this node's transport"
    )]
    pub local_peer: Option<SocketAddr>,

    #[arg(long, help = "Allow plain TCP when the peer address is private/local")]
    pub allow_plain_tcp: bool,

    #[arg(long)]
    pub id: Option<String>,

    #[arg(long, help = "Do not save this route for daemon restart recovery")]
    pub volatile: bool,

    #[arg(
        long,
        help = "Print the daemon commands that would be sent, without changing routes"
    )]
    pub dry_run: bool,

    #[arg(
        long,
        help = "Print the expanded route plan locally, including topology and preflight hints, without changing routes"
    )]
    #[serde(default)]
    pub explain: bool,

    #[arg(long, help = "Emit machine-readable JSON output")]
    #[serde(default)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RouteDirection {
    LocalUsesRemote,
    RemoteUsesLocal,
}

impl From<RouteDirection> for model::RouteDirection {
    fn from(value: RouteDirection) -> Self {
        match value {
            RouteDirection::LocalUsesRemote => Self::LocalUsesRemote,
            RouteDirection::RemoteUsesLocal => Self::RemoteUsesLocal,
        }
    }
}

impl From<model::RouteDirection> for RouteDirection {
    fn from(value: model::RouteDirection) -> Self {
        match value {
            model::RouteDirection::LocalUsesRemote => Self::LocalUsesRemote,
            model::RouteDirection::RemoteUsesLocal => Self::RemoteUsesLocal,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RouteConnectMode {
    Auto,
    Direct,
    ReverseLink,
}

impl From<RouteConnectMode> for model::RouteConnectMode {
    fn from(value: RouteConnectMode) -> Self {
        match value {
            RouteConnectMode::Auto => Self::Auto,
            RouteConnectMode::Direct => Self::Direct,
            RouteConnectMode::ReverseLink => Self::ReverseLink,
        }
    }
}

impl From<model::RouteConnectMode> for RouteConnectMode {
    fn from(value: model::RouteConnectMode) -> Self {
        match value {
            model::RouteConnectMode::Auto => Self::Auto,
            model::RouteConnectMode::Direct => Self::Direct,
            model::RouteConnectMode::ReverseLink => Self::ReverseLink,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RouteWorkloadHint {
    Large,
    Concurrent,
    Mixed,
}

impl From<RouteWorkloadHint> for model::WorkloadHint {
    fn from(value: RouteWorkloadHint) -> Self {
        match value {
            RouteWorkloadHint::Large => Self::Large,
            RouteWorkloadHint::Concurrent => Self::Concurrent,
            RouteWorkloadHint::Mixed => Self::Mixed,
        }
    }
}

impl From<model::WorkloadHint> for RouteWorkloadHint {
    fn from(value: model::WorkloadHint) -> Self {
        match value {
            model::WorkloadHint::Large => Self::Large,
            model::WorkloadHint::Concurrent => Self::Concurrent,
            model::WorkloadHint::Mixed => Self::Mixed,
        }
    }
}
