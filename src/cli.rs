use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    str::FromStr,
};

use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

use crate::{control_socket, peer_transport};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TcpTarget {
    pub host: String,
    pub port: u16,
}

impl std::fmt::Display for TcpTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

impl FromStr for TcpTarget {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Some(rest) = value.strip_prefix('[') {
            let Some((host, tail)) = rest.split_once("]:") else {
                return Err("expected [ipv6]:port".to_string());
            };
            let port = tail
                .parse::<u16>()
                .map_err(|_| format!("invalid TCP target port {tail:?}"))?;
            return Ok(Self {
                host: host.to_string(),
                port,
            });
        }
        let Some((host, port)) = value.rsplit_once(':') else {
            return Err("expected host:port".to_string());
        };
        if host.is_empty() {
            return Err("TCP target host cannot be empty".to_string());
        }
        let port = port
            .parse::<u16>()
            .map_err(|_| format!("invalid TCP target port {port:?}"))?;
        Ok(Self {
            host: host.to_string(),
            port,
        })
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "ssh_proxy",
    version,
    about = "SOCKS5H/HTTP TCP/UDP proxy over peer node transports"
)]
pub struct Cli {
    #[arg(
        long,
        global = true,
        default_value = "info",
        help = "Log filter, e.g. trace,debug,ssh_proxy=trace"
    )]
    pub log: String,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    #[command(about = "Run local SOCKS5H/HTTP proxy and connect a remote peer")]
    Proxy(ProxyArgs),
    #[command(about = "Create a daemon-owned proxy route with automatic defaults")]
    Route(RouteArgs),
    #[command(about = "Run a SOCKS5H listener on the SSH host and use this machine as egress")]
    Reverse(ReverseArgs),
    #[command(about = "Run the remote helper. Usually started by the local proxy over SSH")]
    Remote(RemoteArgs),
    #[command(about = "Run or control the symmetric ssh_proxy node daemon")]
    Node(NodeArgs),
    #[command(
        about = "Copy this executable to the target host and optionally install a daemon command"
    )]
    InstallRemote(InstallRemoteArgs),
    #[command(about = "Show local TOML configuration path or sample")]
    Config(ConfigArgs),
    #[command(about = "Talk to a running local ssh_proxy controller")]
    Control(ControlArgs),
    #[command(about = "Run the local daemon control plane")]
    Daemon(DaemonArgs),
    #[command(about = "Manage the remote host helper service through SSH")]
    Host(HostArgs),
    #[command(about = "Install or print local service integration commands")]
    Service(ServiceArgs),
}

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

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RouteConnectMode {
    Auto,
    Direct,
    ReverseLink,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RouteWorkloadHint {
    Large,
    Concurrent,
    Mixed,
}

#[derive(Debug, Clone, Parser)]
pub struct ReverseArgs {
    #[arg(help = "SSH target, e.g. host, user@host, or an alias from ~/.ssh/config")]
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
        help = "Optional upstream proxy used by this local egress side, e.g. http://127.0.0.1:<proxy-port>"
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

    #[arg(long)]
    pub no_reconnect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReverseTaskArgs {
    pub target: String,
    #[serde(default = "default_reverse_listen")]
    pub remote_listen: SocketAddr,
    pub tcp_target: Option<TcpTarget>,
    #[serde(default)]
    pub ssh_args: Vec<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    #[serde(default)]
    pub identity: Vec<PathBuf>,
    pub config: Option<PathBuf>,
    pub known_hosts: Option<PathBuf>,
    #[serde(default)]
    pub accept_new: bool,
    #[serde(default)]
    pub insecure_ignore_host_key: bool,
    #[serde(default)]
    pub jump: Vec<String>,
    pub remote_path: Option<String>,
    pub remote_bin: Option<PathBuf>,
    #[serde(default)]
    pub deploy: DeployMode,
    #[serde(default)]
    pub remote_os: RemoteOs,
    pub egress_proxy: Option<String>,
    #[serde(default = "default_reconnect_delay")]
    pub reconnect_delay_secs: u64,
    #[serde(default = "default_reconnect_max_delay")]
    pub reconnect_max_delay_secs: u64,
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,
    #[serde(default)]
    pub transport_pool_source: Option<String>,
    #[serde(default)]
    pub transport_pool_reason: Option<String>,
    #[serde(default)]
    pub no_reconnect: bool,
}

impl From<ReverseTaskArgs> for ReverseArgs {
    fn from(args: ReverseTaskArgs) -> Self {
        Self {
            target: args.target,
            remote_listen: args.remote_listen,
            tcp_target: args.tcp_target,
            ssh_args: args.ssh_args,
            user: args.user,
            port: args.port,
            identity: args.identity,
            config: args.config,
            known_hosts: args.known_hosts,
            accept_new: args.accept_new,
            insecure_ignore_host_key: args.insecure_ignore_host_key,
            jump: args.jump,
            remote_path: args.remote_path,
            remote_bin: args.remote_bin,
            deploy: args.deploy,
            remote_os: args.remote_os,
            egress_proxy: args.egress_proxy,
            reconnect_delay_secs: args.reconnect_delay_secs,
            reconnect_max_delay_secs: args.reconnect_max_delay_secs,
            connect_timeout_secs: args.connect_timeout_secs,
            transport_pool_size: 1,
            no_reconnect: args.no_reconnect,
        }
    }
}

fn default_reverse_listen() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 1080))
}

fn default_reconnect_delay() -> u64 {
    5
}

fn default_reconnect_max_delay() -> u64 {
    60
}

fn default_connect_timeout() -> u64 {
    30
}

#[derive(Debug, Clone, Parser)]
pub struct RemoteArgs {
    #[arg(long, help = "Use stdio framed transport")]
    pub stdio: bool,

    #[arg(
        long,
        help = "Run a SOCKS5H listener on the remote side using stdio bridge"
    )]
    pub reverse_socks: Option<SocketAddr>,

    #[arg(long, help = "Experimental TCP framed transport listen address")]
    pub tcp_listen: Option<SocketAddr>,

    #[arg(long, help = "Shared token required by TCP framed transport")]
    pub token: Option<String>,
}

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

#[derive(Debug, Clone, Parser)]
pub struct InstallRemoteArgs {
    pub target: String,

    #[arg(long = "ssh-arg")]
    pub ssh_args: Vec<String>,

    #[arg(
        long,
        help = "Rejected when using russh; kept to produce a clear migration error"
    )]
    pub ssh_command: Option<String>,

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

    #[arg(long, hide = true)]
    pub local_node_id: Option<String>,

    #[arg(long, hide = true)]
    pub local_node_name: Option<String>,

    #[arg(long, hide = true)]
    pub local_control_endpoint: Option<String>,

    #[arg(long, hide = true)]
    pub local_transport: Option<SocketAddr>,

    #[arg(long, hide = true)]
    pub remote_node_id: Option<String>,

    #[arg(long, hide = true)]
    pub remote_node_name: Option<String>,

    #[arg(
        long,
        help = "Direct TLS framed transport listener installed on the remote node"
    )]
    pub remote_tls_transport: Option<SocketAddr>,

    #[arg(
        long,
        help = "Direct QUIC framed transport listener installed on the remote node"
    )]
    pub remote_quic_transport: Option<SocketAddr>,

    #[arg(long, help = "Remote PEM certificate chain path for TLS/QUIC listener")]
    pub remote_tls_cert: Option<String>,

    #[arg(long, help = "Remote PEM private key path for TLS/QUIC listener")]
    pub remote_tls_key: Option<String>,

    #[arg(
        long,
        help = "Remote PEM CA/root used to verify TLS peer client certificates"
    )]
    pub remote_tls_client_ca: Option<String>,

    #[arg(long, default_value = "none")]
    pub persist: PersistMode,
}

#[derive(Debug, Clone, Parser)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Debug, Clone, Parser)]
pub struct ControlArgs {
    #[arg(short, long, default_value = "127.0.0.1:1081")]
    pub addr: SocketAddr,

    #[arg(
        long,
        help = "Control endpoint: tcp://host:port, unix:///path.sock, or npipe://name"
    )]
    pub endpoint: Option<String>,

    #[command(subcommand)]
    pub command: ControlCommand,
}

#[derive(Debug, Clone, Parser)]
pub struct DaemonArgs {
    #[arg(short, long, default_value = "127.0.0.1:1081")]
    pub control_listen: SocketAddr,

    #[arg(
        long,
        help = "Control endpoint to listen on: tcp://host:port, unix:///path.sock, or npipe://name"
    )]
    pub control: Option<String>,
}

#[derive(Debug, Clone, Parser)]
pub struct HostArgs {
    pub target: String,

    #[command(subcommand)]
    pub command: HostCommand,

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

    #[arg(
        long,
        help = "Direct TLS framed transport listener installed on the remote node"
    )]
    pub remote_tls_transport: Option<SocketAddr>,

    #[arg(
        long,
        help = "Direct QUIC framed transport listener installed on the remote node"
    )]
    pub remote_quic_transport: Option<SocketAddr>,

    #[arg(long, help = "Remote PEM certificate chain path for TLS/QUIC listener")]
    pub remote_tls_cert: Option<String>,

    #[arg(long, help = "Remote PEM private key path for TLS/QUIC listener")]
    pub remote_tls_key: Option<String>,

    #[arg(
        long,
        help = "Remote PEM CA/root used to verify TLS peer client certificates"
    )]
    pub remote_tls_client_ca: Option<String>,

    #[arg(long, default_value = "auto")]
    pub persist: PersistMode,
}

#[derive(Debug, Clone, Parser)]
pub struct HostExecArgs {
    #[arg(
        long,
        help = "Read a Unix shell script from stdin and execute it with sh -s"
    )]
    pub stdin: bool,

    #[arg(
        long,
        default_value = "host exec",
        help = "Human-readable label for diagnostics"
    )]
    pub label: String,

    #[arg(
        long,
        default_value_t = 30,
        help = "Remote execution timeout in seconds"
    )]
    pub timeout_secs: u64,

    #[arg(long, help = "Emit machine-readable JSON output")]
    pub json: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct ServiceArgs {
    #[arg(long, value_enum, default_value = "auto")]
    pub scope: ServiceScope,

    #[arg(
        long,
        help = "Daemon control endpoint used in installed service command"
    )]
    pub control: Option<String>,

    #[arg(
        long,
        help = "Framed transport listener used by peer node daemons. Defaults to a user-scoped localhost port"
    )]
    pub transport: Option<SocketAddr>,

    #[arg(long, help = "Disable the default local framed transport listener")]
    pub no_transport: bool,

    #[arg(long, help = "Shared token required by framed transport clients")]
    pub token: Option<String>,

    #[arg(long, help = "Direct TLS framed transport listener for proxy peers")]
    pub tls_transport: Option<SocketAddr>,

    #[arg(long, help = "Direct QUIC framed transport listener for proxy peers")]
    pub quic_transport: Option<SocketAddr>,

    #[arg(long, help = "PEM certificate chain for TLS/QUIC listeners")]
    pub tls_cert: Option<PathBuf>,

    #[arg(long, help = "PEM private key for TLS/QUIC listeners")]
    pub tls_key: Option<PathBuf>,

    #[arg(long, help = "PEM CA/root used to verify TLS peer client certificates")]
    pub tls_client_ca: Option<PathBuf>,

    #[arg(
        long = "report-to",
        help = "Peer node control endpoint to receive daemon status reports"
    )]
    pub report_to: Vec<String>,

    #[arg(long, help = "Directory to copy the local service binary into")]
    pub install_dir: Option<PathBuf>,

    #[arg(
        long,
        help = "Do not copy the binary; service uses the current executable"
    )]
    pub no_copy: bool,

    #[arg(long, help = "Emit machine-readable JSON output")]
    pub json: bool,

    #[command(subcommand)]
    pub command: ServiceCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ConfigCommand {
    Path,
    Sample,
    Init {
        #[arg(long, help = "Overwrite an existing config file")]
        force: bool,
    },
    Show,
    Inspect,
    ExportDescriptor,
    ImportDescriptor(ConfigImportDescriptorArgs),
    Profiles,
    Peers,
    ProfileSet(ConfigProfileSetArgs),
    ProfileRemove {
        name: String,
    },
    Token {
        #[arg(long, help = "Replace the existing daemon token")]
        rotate: bool,
    },
    CertImport(ConfigCertImportArgs),
}

#[derive(Debug, Clone, Parser)]
pub struct ConfigImportDescriptorArgs {
    pub alias: String,

    #[arg(default_value = "-", help = "Descriptor JSON path, or '-' for stdin")]
    pub path: String,

    #[arg(
        long,
        help = "SSH/profile target to record; defaults to descriptor target or alias"
    )]
    pub target: Option<String>,

    #[arg(
        long,
        help = "Optional peer control/transport token supplied out of band"
    )]
    pub token: Option<String>,

    #[arg(long, default_value = "descriptor-import")]
    pub trust: String,
}

#[derive(Debug, Clone, Parser)]
pub struct ConfigProfileSetArgs {
    pub name: String,

    #[arg(long)]
    pub target: Option<String>,

    #[arg(long)]
    pub user: Option<String>,

    #[arg(short = 'p', long)]
    pub port: Option<u16>,

    #[arg(short = 'i', long)]
    pub identity: Vec<PathBuf>,

    #[arg(short = 'F', long)]
    pub ssh_config: Option<PathBuf>,

    #[arg(long)]
    pub known_hosts: Option<PathBuf>,

    #[arg(long)]
    pub accept_new: bool,

    #[arg(long)]
    pub no_accept_new: bool,

    #[arg(long, alias = "no-known-hosts")]
    pub insecure_ignore_host_key: bool,

    #[arg(long)]
    pub no_insecure_ignore_host_key: bool,

    #[arg(short = 'J', long = "jump")]
    pub jump: Vec<String>,

    #[arg(long)]
    pub listen: Option<SocketAddr>,

    #[arg(long, value_name = "HOST:PORT")]
    pub tcp_target: Option<TcpTarget>,

    #[arg(long)]
    pub remote_transport: Option<String>,

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

    #[arg(long)]
    pub remote_name: Option<String>,

    #[arg(long)]
    pub remote_client_cert: Option<PathBuf>,

    #[arg(long)]
    pub remote_client_key: Option<PathBuf>,

    #[arg(long)]
    pub remote_token: Option<String>,

    #[arg(long)]
    pub egress_proxy: Option<String>,

    #[arg(long)]
    pub allow_plain_tcp: bool,

    #[arg(long)]
    pub no_allow_plain_tcp: bool,

    #[arg(long)]
    pub transport_pool_size: Option<usize>,

    #[arg(
        long,
        value_enum,
        help = "Workload hint for automatic transport pool sizing: large, concurrent, or mixed"
    )]
    pub workload_hint: Option<RouteWorkloadHint>,

    #[arg(long)]
    pub ssh_session_pool_size: Option<usize>,

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
}

#[derive(Debug, Clone, Parser)]
pub struct ConfigCertImportArgs {
    pub name: String,

    #[arg(
        long,
        help = "Attach imported remote/client cert paths to this profile"
    )]
    pub profile: Option<String>,

    #[arg(long, help = "Attach imported server cert paths to daemon config")]
    pub daemon: bool,

    #[arg(long, help = "CA/root certificate used to verify a remote peer")]
    pub remote_ca: Option<PathBuf>,

    #[arg(long, help = "Client certificate presented for mTLS")]
    pub client_cert: Option<PathBuf>,

    #[arg(long, help = "Client private key presented for mTLS")]
    pub client_key: Option<PathBuf>,

    #[arg(long, help = "Server certificate used by local TLS/QUIC listeners")]
    pub tls_cert: Option<PathBuf>,

    #[arg(long, help = "Server private key used by local TLS/QUIC listeners")]
    pub tls_key: Option<PathBuf>,

    #[arg(long, help = "CA/root used to verify connecting TLS clients")]
    pub tls_client_ca: Option<PathBuf>,

    #[arg(
        long,
        help = "Overwrite files already present in the certificate store"
    )]
    pub overwrite: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ControlCommand {
    Status,
    Shutdown,
    Connect { profile: String },
    Disconnect { profile: String },
}

#[derive(Debug, Clone, Subcommand)]
pub enum HostCommand {
    Status,
    NodeStatus,
    NodeDescriptor,
    NodeLinks,
    NodeForward(NodeForwardArgs),
    NodeReverse(NodeReverseArgs),
    NodeStopRoute {
        id: String,
    },
    NodeRestartRoute {
        id: String,
    },
    NodeRoutes,
    NodeConnect {
        profile: String,
    },
    NodeDisconnect {
        profile: String,
    },
    Exec(HostExecArgs),
    Start,
    Stop,
    Restart,
    Logs {
        #[arg(short, long, default_value_t = 80)]
        lines: usize,
    },
    Clean,
    Doctor,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ServiceCommand {
    Print,
    Install,
    Uninstall,
    Start,
    Stop,
    Status,
}

#[derive(Debug, Clone, Copy, Default, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DeployMode {
    #[default]
    #[serde(alias = "Auto")]
    Auto,
    #[serde(alias = "Always")]
    Always,
    #[serde(alias = "Never")]
    Never,
}

#[derive(Debug, Clone, Copy, Default, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RemoteOs {
    #[default]
    #[serde(alias = "Auto")]
    Auto,
    #[serde(alias = "Unix")]
    Unix,
    #[serde(alias = "Windows")]
    Windows,
}

#[derive(Debug, Clone, Copy, Default, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RemoteTransport {
    #[default]
    #[serde(alias = "Auto")]
    Auto,
    #[serde(alias = "SshNative", alias = "ssh_native", alias = "ssh-native")]
    SshNative,
    #[serde(alias = "QuicNative", alias = "quic_native", alias = "native-quic")]
    QuicNative,
    #[serde(alias = "Quic")]
    Quic,
    #[serde(alias = "TlsTcp", alias = "tls_tcp", alias = "tls")]
    TlsTcp,
    #[serde(
        alias = "PlainTcp",
        alias = "DirectTcp",
        alias = "plain_tcp",
        alias = "direct_tcp"
    )]
    PlainTcp,
    #[serde(alias = "Exec")]
    Exec,
    #[serde(alias = "Tcp")]
    Tcp,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum PersistMode {
    None,
    Auto,
    Systemd,
    Nohup,
    Launchd,
    Schtasks,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum ServiceScope {
    Auto,
    User,
    System,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_exec_accepts_stdin_json_shape() {
        let cli = Cli::try_parse_from([
            "ssh_proxy",
            "host",
            "edge",
            "exec",
            "--stdin",
            "--label",
            "remote setup",
            "--timeout-secs",
            "7",
            "--json",
        ])
        .unwrap();

        match cli.command {
            Commands::Host(args) => match args.command {
                HostCommand::Exec(exec) => {
                    assert_eq!(args.target, "edge");
                    assert!(exec.stdin);
                    assert_eq!(exec.label, "remote setup");
                    assert_eq!(exec.timeout_secs, 7);
                    assert!(exec.json);
                }
                other => panic!("unexpected host command: {other:?}"),
            },
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn route_accepts_json_flag() {
        let cli = Cli::try_parse_from([
            "ssh_proxy",
            "route",
            "edge",
            "--direction",
            "remote-uses-local",
            "--json",
        ])
        .unwrap();

        match cli.command {
            Commands::Route(args) => {
                assert_eq!(args.target, "edge");
                assert_eq!(args.direction, RouteDirection::RemoteUsesLocal);
                assert!(args.json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn service_accepts_json_flag() {
        let cli = Cli::try_parse_from(["ssh_proxy", "service", "--json", "status"]).unwrap();

        match cli.command {
            Commands::Service(args) => {
                assert!(args.json);
                assert!(matches!(args.command, ServiceCommand::Status));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn node_control_accepts_json_flag() {
        let cli =
            Cli::try_parse_from(["ssh_proxy", "node", "control", "--json", "status"]).unwrap();

        match cli.command {
            Commands::Node(args) => match args.command {
                NodeCommand::Control(control) => {
                    assert!(control.json);
                    assert!(matches!(control.command, NodeControlCommand::Status));
                }
                other => panic!("unexpected node command: {other:?}"),
            },
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
