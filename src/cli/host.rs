use std::{net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand};

use super::{NodeForwardArgs, NodeReverseArgs, PersistMode, RemoteOs};

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
