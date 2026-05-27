use std::{net::IpAddr, path::PathBuf};

use clap::{Parser, Subcommand, ValueEnum};

use crate::control_socket;

use super::{NodeDaemonArgs, RouteConnectMode};

#[derive(Debug, Clone, Parser)]
pub struct DaemonArgs {
    #[arg(long, value_enum, default_value = "system")]
    pub scope: DaemonScope,

    #[arg(long, help = "Emit machine-readable JSON output")]
    pub json: bool,

    #[command(subcommand)]
    pub command: DaemonCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum DaemonCommand {
    #[command(about = "Install the privileged ssh_proxy daemon service")]
    Install {
        #[arg(long, help = "Allow an interactive elevation prompt")]
        elevate: bool,
        #[arg(long, help = "Do not copy the binary into the service install dir")]
        no_copy: bool,
    },
    #[command(about = "Uninstall the daemon service")]
    Uninstall,
    #[command(about = "Start the daemon service")]
    Start,
    #[command(about = "Stop the daemon service")]
    Stop,
    #[command(about = "Show daemon health and selected control endpoint")]
    Status,
    #[command(about = "Submit a daemon self-update job")]
    Update {
        #[arg(long, help = "Replacement ssh_proxy binary staged by the caller")]
        source: Option<PathBuf>,
    },
    #[command(about = "Run the daemon control plane in the foreground")]
    Serve(NodeDaemonArgs),
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum DaemonScope {
    Auto,
    User,
    System,
}

#[derive(Debug, Clone, Parser)]
pub struct UpArgs {
    #[arg(long, help = "SSH target alias or user@host")]
    pub target: String,

    #[arg(long, help = "Local proxy URL used as the egress side")]
    pub local_proxy: String,

    #[arg(long, help = "Workspace/session id used to deduplicate routes")]
    pub workspace: Option<String>,

    #[arg(long, default_value = "127.0.0.1", help = "Remote bind address")]
    pub remote_bind: IpAddr,

    #[arg(long, default_value_t = 17890, help = "Preferred remote proxy port")]
    pub remote_port: u16,

    #[arg(long, value_enum, default_value = "reverse-link")]
    pub connect_mode: RouteConnectMode,

    #[arg(long, default_value_t = control_socket::default_endpoint_string())]
    pub endpoint: String,

    #[arg(long, help = "Token for TCP control endpoints")]
    pub token: Option<String>,

    #[arg(long, help = "Stable route id; defaults to the workspace or target")]
    pub id: Option<String>,

    #[arg(
        long = "workspace-path",
        help = "Remote workspace path used for daemon-owned Git workspace settings"
    )]
    pub workspace_paths: Vec<String>,

    #[arg(long, default_value = ".vscode-server", help = "Remote VS Code server directory")]
    pub server_dir: String,

    #[arg(
        long,
        default_value = "localhost,127.0.0.1,::1",
        help = "no_proxy value applied to daemon-owned remote setup"
    )]
    pub no_proxy: String,

    #[arg(
        long,
        default_value = "override",
        help = "proxy support mode written by daemon-owned remote setup"
    )]
    pub proxy_support: String,

    #[arg(long, help = "Skip remote VS Code machine settings application")]
    pub no_remote_machine_settings: bool,

    #[arg(long, help = "Skip remote terminal environment setup")]
    pub no_terminal_env: bool,

    #[arg(long, help = "Skip remote server-env-setup management")]
    pub no_server_env: bool,

    #[arg(long, help = "Skip remote Git proxy config management")]
    pub no_git: bool,

    #[arg(long, help = "Skip global Git proxy config management")]
    pub no_git_global: bool,

    #[arg(long, help = "Skip workspace Git proxy config management")]
    pub no_git_workspace: bool,

    #[arg(long, help = "Skip force-replacing existing Git proxy config")]
    pub no_git_force_override: bool,

    #[arg(long, help = "Skip remote proxy status file management")]
    pub no_remote_status_file: bool,

    #[arg(long, help = "Skip remote port readiness verification")]
    pub no_verify_remote_port: bool,

    #[arg(long, help = "Do not restore this proxy session after daemon restart")]
    pub volatile: bool,

    #[arg(long, help = "Emit machine-readable JSON output")]
    pub json: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct DownArgs {
    #[arg(long, help = "SSH target alias or user@host")]
    pub target: Option<String>,

    #[arg(long, help = "Workspace/session id used to derive the route id")]
    pub workspace: Option<String>,

    #[arg(long, help = "Route id to stop")]
    pub route_id: Option<String>,

    #[arg(long, default_value_t = control_socket::default_endpoint_string())]
    pub endpoint: String,

    #[arg(long, help = "Token for TCP control endpoints")]
    pub token: Option<String>,

    #[arg(long, help = "Emit machine-readable JSON output")]
    pub json: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct StatusArgs {
    #[arg(long, help = "Filter status to one SSH target")]
    pub target: Option<String>,

    #[arg(long, help = "Workspace/session id to inspect")]
    pub workspace: Option<String>,

    #[arg(long, default_value_t = control_socket::default_endpoint_string())]
    pub endpoint: String,

    #[arg(long, help = "Token for TCP control endpoints")]
    pub token: Option<String>,

    #[arg(long, help = "Emit machine-readable JSON output")]
    pub json: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct EventsArgs {
    #[arg(long, help = "Job id to inspect")]
    pub job: Option<String>,

    #[arg(long, default_value_t = control_socket::default_endpoint_string())]
    pub endpoint: String,

    #[arg(long, help = "Token for TCP control endpoints")]
    pub token: Option<String>,

    #[arg(long, help = "Emit machine-readable JSON output")]
    pub json: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct DoctorArgs {
    #[arg(long, default_value_t = control_socket::default_endpoint_string())]
    pub endpoint: String,

    #[arg(long, help = "Token for TCP control endpoints")]
    pub token: Option<String>,

    #[arg(long, help = "Emit machine-readable JSON output")]
    pub json: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct VscodeArgs {
    #[command(subcommand)]
    pub command: VscodeCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum VscodeCommand {
    #[command(about = "Ensure a VS Code Remote SSH proxy session")]
    Up(VscodeUpArgs),
    #[command(about = "Show status for a VS Code workspace session")]
    Status(VscodeStatusArgs),
    #[command(about = "Apply remote VS Code/Git/env proxy settings")]
    ApplySettings(VscodeApplySettingsArgs),
    #[command(about = "Collect VS Code-focused daemon diagnostics")]
    Diagnose(VscodeDiagnoseArgs),
}

#[derive(Debug, Clone, Parser)]
pub struct VscodeUpArgs {
    #[arg(long)]
    pub target: String,

    #[arg(long)]
    pub workspace: String,

    #[arg(long)]
    pub local_proxy: String,

    #[arg(long, default_value = "127.0.0.1")]
    pub remote_bind: IpAddr,

    #[arg(long, default_value_t = 17890)]
    pub remote_port: u16,

    #[arg(long, value_enum, default_value = "reverse-link")]
    pub connect_mode: RouteConnectMode,

    #[arg(
        long = "workspace-path",
        help = "Remote workspace path used for daemon-owned Git workspace settings"
    )]
    pub workspace_paths: Vec<String>,

    #[arg(long, default_value = ".vscode-server")]
    pub server_dir: String,

    #[arg(long, default_value = "localhost,127.0.0.1,::1")]
    pub no_proxy: String,

    #[arg(long, default_value = "override")]
    pub proxy_support: String,

    #[arg(long)]
    pub no_remote_machine_settings: bool,

    #[arg(long)]
    pub no_terminal_env: bool,

    #[arg(long)]
    pub no_server_env: bool,

    #[arg(long)]
    pub no_git: bool,

    #[arg(long)]
    pub no_git_global: bool,

    #[arg(long)]
    pub no_git_workspace: bool,

    #[arg(long)]
    pub no_git_force_override: bool,

    #[arg(long)]
    pub no_remote_status_file: bool,

    #[arg(long)]
    pub no_verify_remote_port: bool,

    #[arg(long, default_value_t = control_socket::default_endpoint_string())]
    pub endpoint: String,

    #[arg(long)]
    pub token: Option<String>,

    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct VscodeStatusArgs {
    #[arg(long)]
    pub workspace: Option<String>,

    #[arg(long)]
    pub target: Option<String>,

    #[arg(long, default_value_t = control_socket::default_endpoint_string())]
    pub endpoint: String,

    #[arg(long)]
    pub token: Option<String>,

    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct VscodeApplySettingsArgs {
    #[arg(long)]
    pub target: String,

    #[arg(long)]
    pub workspace: String,

    #[arg(long)]
    pub proxy_url: String,

    #[arg(long, default_value_t = control_socket::default_endpoint_string())]
    pub endpoint: String,

    #[arg(long)]
    pub token: Option<String>,

    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct VscodeDiagnoseArgs {
    #[arg(long)]
    pub workspace: Option<String>,

    #[arg(long, default_value_t = control_socket::default_endpoint_string())]
    pub endpoint: String,

    #[arg(long)]
    pub token: Option<String>,

    #[arg(long)]
    pub json: bool,
}

impl DaemonScope {
    pub fn as_service_scope(self) -> super::ServiceScope {
        match self {
            Self::Auto => super::ServiceScope::Auto,
            Self::User => super::ServiceScope::User,
            Self::System => super::ServiceScope::System,
        }
    }
}

impl VscodeUpArgs {
    pub fn into_up_args(self) -> UpArgs {
        UpArgs {
            target: self.target,
            local_proxy: self.local_proxy,
            workspace: Some(self.workspace),
            remote_bind: self.remote_bind,
            remote_port: self.remote_port,
            connect_mode: self.connect_mode,
            endpoint: self.endpoint,
            token: self.token,
            id: None,
            workspace_paths: self.workspace_paths,
            server_dir: self.server_dir,
            no_proxy: self.no_proxy,
            proxy_support: self.proxy_support,
            no_remote_machine_settings: self.no_remote_machine_settings,
            no_terminal_env: self.no_terminal_env,
            no_server_env: self.no_server_env,
            no_git: self.no_git,
            no_git_global: self.no_git_global,
            no_git_workspace: self.no_git_workspace,
            no_git_force_override: self.no_git_force_override,
            no_remote_status_file: self.no_remote_status_file,
            no_verify_remote_port: self.no_verify_remote_port,
            volatile: true,
            json: self.json,
        }
    }
}
