use std::net::SocketAddr;

use clap::{Parser, Subcommand};

#[derive(Debug, Clone, Parser)]
pub struct RemoteArgs {
    #[command(subcommand)]
    pub command: Option<RemoteCommand>,

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

#[derive(Debug, Clone, Subcommand)]
pub enum RemoteCommand {
    #[command(hide = true, about = "Run a structured remote admin intent")]
    Admin {
        #[arg(help = "RemoteAdminIntent JSON; reads stdin when omitted")]
        json: Option<String>,
    },
}
