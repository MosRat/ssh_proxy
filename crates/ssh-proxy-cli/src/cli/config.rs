use std::{net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand};

use super::{RouteWorkloadHint, TcpTarget};

#[derive(Debug, Clone, Parser)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
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
