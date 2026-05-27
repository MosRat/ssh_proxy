use std::net::SocketAddr;

use clap::{Parser, Subcommand};

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
pub struct ControllerDaemonArgs {
    #[arg(short, long, default_value = "127.0.0.1:1081")]
    pub control_listen: SocketAddr,

    #[arg(
        long,
        help = "Control endpoint to listen on: tcp://host:port, unix:///path.sock, or npipe://name"
    )]
    pub control: Option<String>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ControlCommand {
    Status,
    Shutdown,
    Connect { profile: String },
    Disconnect { profile: String },
}
