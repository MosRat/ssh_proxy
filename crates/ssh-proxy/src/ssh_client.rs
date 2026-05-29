use std::path::PathBuf;

use anyhow::{Result, bail};
use russh::client;

use crate::cli;

pub use ssh_proxy_ssh::{ExecOutput, Target, resolve_target};

pub struct Client(ssh_proxy_ssh::Client);

impl Client {
    pub async fn connect_proxy_args(args: &cli::ProxyArgs) -> Result<Self> {
        if args.ssh_command.is_some() {
            reject_external_ssh_command()?;
        }
        connect_target(
            &args.target,
            &args.ssh_args,
            args.user.clone(),
            args.port,
            args.identity.clone(),
            args.config.clone(),
            args.known_hosts.clone(),
            args.accept_new,
            args.insecure_ignore_host_key,
            args.jump.clone(),
        )
        .await
    }

    pub async fn connect_install_args(args: &cli::InstallRemoteArgs) -> Result<Self> {
        if args.ssh_command.is_some() {
            reject_external_ssh_command()?;
        }
        connect_target(
            &args.target,
            &args.ssh_args,
            args.user.clone(),
            args.port,
            args.identity.clone(),
            args.config.clone(),
            args.known_hosts.clone(),
            args.accept_new,
            args.insecure_ignore_host_key,
            args.jump.clone(),
        )
        .await
    }

    pub fn target(&self) -> &Target {
        self.0.target()
    }

    pub async fn exec_stream(&self, command: String) -> Result<russh::ChannelStream<client::Msg>> {
        self.0.exec_stream(command).await
    }

    pub async fn direct_tcpip_stream(
        &self,
        host: String,
        port: u16,
    ) -> Result<russh::ChannelStream<client::Msg>> {
        self.0.direct_tcpip_stream(host, port).await
    }

    pub async fn exec_upload(&self, command: String, bytes: Vec<u8>) -> Result<()> {
        self.0.exec_upload(command, bytes).await
    }

    pub async fn exec_output(&self, command: String) -> Result<String> {
        self.0.exec_output(command).await
    }

    pub async fn exec_capture(
        &self,
        command: String,
        stdin: Option<Vec<u8>>,
    ) -> Result<ExecOutput> {
        self.0.exec_capture(command, stdin).await
    }
}

pub fn resolve_route_target(args: &cli::RouteArgs) -> Result<Target> {
    resolve_target(
        &args.target,
        &args.ssh_args,
        args.user.clone(),
        args.ssh_port,
        args.identity.clone(),
        args.config.clone(),
        args.known_hosts.clone(),
        args.accept_new,
        args.insecure_ignore_host_key,
        args.jump.clone(),
    )
}

async fn connect_target(
    target: &str,
    ssh_args: &[String],
    user: Option<String>,
    port: Option<u16>,
    identities: Vec<PathBuf>,
    config: Option<PathBuf>,
    known_hosts: Option<PathBuf>,
    accept_new: bool,
    insecure_ignore_host_key: bool,
    jump: Vec<String>,
) -> Result<Client> {
    let target = resolve_target(
        target,
        ssh_args,
        user,
        port,
        identities,
        config,
        known_hosts,
        accept_new,
        insecure_ignore_host_key,
        jump,
    )?;
    Ok(Client(ssh_proxy_ssh::Client::connect(target).await?))
}

fn reject_external_ssh_command() -> Result<()> {
    bail!(
        "--ssh-command cannot be executed by russh; use --ssh-arg/-F/-i/-p/--user or ~/.ssh/config"
    )
}
