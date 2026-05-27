#![recursion_limit = "256"]

use anyhow::Result;
use clap::Parser;
use mimalloc::MiMalloc;
use tracing::warn;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

mod cli;
mod control_socket;
mod logging;
mod node_daemon;
mod paths;
mod peer_transport;
mod reverse;
mod service;
mod sidecar;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    logging::init(&cli.log)?;

    let app_config = config::AppConfig::load_default().unwrap_or_else(|err| {
        warn!(error = %err, "failed to load local config; using CLI values only");
        config::AppConfig::default()
    });

    match cli.command {
        cli::Commands::Proxy(mut args) => {
            app_config.apply_proxy_defaults(&mut args, None)?;
            controller::run(args).await
        }
        cli::Commands::Route(args) => route::run(args, app_config).await,
        cli::Commands::Reverse(args) => reverse::run(args, app_config).await,
        cli::Commands::Remote(args) => remote::run(args).await,
        cli::Commands::Node(args) => node_daemon::run(args, app_config).await,
        cli::Commands::InstallRemote(mut args) => {
            app_config.apply_install_defaults(&mut args, None)?;
            let mut app_config = app_config;
            let local_identity = app_config.ensure_node_identity()?;
            args.local_node_id = local_identity.node_id;
            args.local_node_name = local_identity.node_name;
            args.local_control_endpoint = app_config.daemon.control_endpoint.clone();
            args.local_transport = app_config.daemon.transport_listen;
            app_config.save_default()?;
            deploy::install_remote(args).await.map(|_| ())
        }
        cli::Commands::Config(args) => config::run(args).await,
        cli::Commands::Control(args) => controller::control(args, app_config).await,
        cli::Commands::Daemon(args) => daemon::daemon(args, app_config).await,
        cli::Commands::Up(args) => daemon::up(args, app_config).await,
        cli::Commands::Down(args) => daemon::down(args, app_config).await,
        cli::Commands::Status(args) => daemon::status(args, app_config).await,
        cli::Commands::Events(args) => daemon::events(args, app_config).await,
        cli::Commands::Doctor(args) => daemon::doctor(args, app_config).await,
        cli::Commands::Vscode(args) => daemon::vscode(args, app_config).await,
        cli::Commands::Host(args) => deploy::host(args, app_config).await,
        cli::Commands::Service(args) => service::run(args, app_config).await,
    }
}

mod config;
mod data_plane;
mod protocol;
mod quic_native;
mod quic_stream;
mod route;
mod ssh_auth;

mod ssh_client;
mod ssh_native;

mod bridge;
mod deploy;

mod controller;
mod daemon;

mod socks;

mod remote;
