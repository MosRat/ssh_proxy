use clap::{Parser, Subcommand};

mod config;
pub use config::{
    ConfigArgs, ConfigCertImportArgs, ConfigCommand, ConfigImportDescriptorArgs,
    ConfigProfileSetArgs,
};
mod control;
pub use control::{ControlArgs, ControlCommand, ControllerDaemonArgs};
mod daemon;
pub use daemon::{
    DaemonArgs, DaemonCommand, DaemonScope, DoctorArgs, DownArgs, EventsArgs, StatusArgs, UpArgs,
    VscodeArgs, VscodeCommand,
};
mod host;
pub use host::{HostArgs, HostCommand, HostExecArgs};
mod install;
pub use install::InstallRemoteArgs;
mod node;
pub use node::{
    NodeArgs, NodeCommand, NodeControlArgs, NodeControlCommand, NodeControlScope, NodeDaemonArgs,
    NodeForwardArgs, NodeReverseArgs, PeerBootstrapArgs,
};
mod proxy;
pub use proxy::ProxyArgs;
mod reverse;
pub use reverse::{ReverseArgs, ReverseTaskArgs};
mod remote;
pub use remote::RemoteArgs;
mod route;
pub use route::{RouteArgs, RouteConnectMode, RouteDirection, RouteWorkloadHint};
mod service;
pub use service::{ServiceArgs, ServiceCommand, ServiceScope};
mod shared;
pub use shared::{DeployMode, PersistMode, RemoteOs, RemoteTransport, TcpTarget};

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
    #[command(about = "Manage or run the Docker-like local ssh_proxy daemon")]
    Daemon(DaemonArgs),
    #[command(about = "Ensure a daemon-owned proxy session")]
    Up(UpArgs),
    #[command(about = "Stop a daemon-owned proxy session")]
    Down(DownArgs),
    #[command(about = "Show daemon-owned proxy state")]
    Status(StatusArgs),
    #[command(about = "Show daemon job events")]
    Events(EventsArgs),
    #[command(about = "Collect daemon diagnostics")]
    Doctor(DoctorArgs),
    #[command(about = "VS Code Remote SSH integration commands")]
    Vscode(VscodeArgs),
    #[command(about = "Manage the remote host helper service through SSH")]
    Host(HostArgs),
    #[command(about = "Install or print local service integration commands")]
    Service(ServiceArgs),
}

#[cfg(test)]
mod tests;
