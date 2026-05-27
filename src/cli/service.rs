use std::{net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand, ValueEnum};

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
pub enum ServiceCommand {
    Print,
    Install,
    Uninstall,
    Start,
    Stop,
    Status,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum ServiceScope {
    Auto,
    User,
    System,
}
