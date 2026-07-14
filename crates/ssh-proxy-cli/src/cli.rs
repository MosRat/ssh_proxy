use clap::{Parser, Subcommand};

mod config;
pub use config::{
    ConfigArgs, ConfigCertImportArgs, ConfigCommand, ConfigImportDescriptorArgs,
    ConfigProfileSetArgs,
};
mod control;
pub use control::{ControlArgs, ControlCommand};
mod daemon;
pub use daemon::{
    DaemonArgs, DaemonCommand, DaemonInstallWorkerArgs, DaemonScope, DoctorArgs, DownArgs,
    EventsArgs, StatusArgs, UpArgs, VscodeArgs, VscodeCommand,
};
mod host;
pub use host::{HostArgs, HostCommand, HostExecArgs};
mod install;
pub use install::InstallRemoteArgs;
mod intents;
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
pub use remote::{RemoteArgs, RemoteCommand};
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
    #[command(hide = true, about = "Internal compatibility proxy entrypoint")]
    Proxy(ProxyArgs),
    #[command(hide = true, about = "Internal route compatibility entrypoint")]
    Route(RouteArgs),
    #[command(hide = true, about = "Internal reverse compatibility entrypoint")]
    Reverse(ReverseArgs),
    #[command(hide = true, about = "Internal remote helper entrypoint")]
    Remote(RemoteArgs),
    #[command(hide = true, about = "Internal node daemon compatibility entrypoint")]
    Node(NodeArgs),
    #[command(
        hide = true,
        about = "Copy this executable to the target host and optionally install a daemon command"
    )]
    InstallRemote(InstallRemoteArgs),
    #[command(hide = true, about = "Internal configuration compatibility entrypoint")]
    Config(ConfigArgs),
    #[command(hide = true, about = "Internal controller compatibility entrypoint")]
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
    #[command(
        hide = true,
        about = "Internal remote host helper compatibility entrypoint"
    )]
    Host(HostArgs),
    #[command(hide = true, about = "Internal service compatibility entrypoint")]
    Service(ServiceArgs),
    #[command(
        name = "daemon-install-worker",
        hide = true,
        about = "Internal elevated daemon install worker"
    )]
    DaemonInstallWorker(DaemonInstallWorkerArgs),
}

#[cfg(test)]
mod tests;
