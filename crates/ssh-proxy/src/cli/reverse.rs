use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;
use serde::{Deserialize, Serialize};

use super::{DeployMode, RemoteOs, TcpTarget};

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
