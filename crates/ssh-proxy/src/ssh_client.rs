use anyhow::Result;
use ssh_proxy_core::intent::SshTargetIntent;

use crate::cli;

pub use ssh_proxy_ssh::{ExecOutput, SshStream, Target};

pub struct Client(ssh_proxy_ssh::Client);

impl Client {
    pub async fn connect_proxy_args(args: &cli::ProxyArgs) -> Result<Self> {
        connect_intent(proxy_ssh_intent(args)).await
    }

    pub async fn connect_install_args(args: &cli::InstallRemoteArgs) -> Result<Self> {
        connect_intent(install_ssh_intent(args)).await
    }

    pub async fn exec_stream(&self, command: String) -> Result<SshStream> {
        self.0.exec_stream(command).await
    }

    pub async fn direct_tcpip_stream(&self, host: String, port: u16) -> Result<SshStream> {
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
    ssh_proxy_ssh::resolve_intent_target(&route_ssh_intent(args))
}

async fn connect_intent(intent: SshTargetIntent) -> Result<Client> {
    Ok(Client(ssh_proxy_ssh::connect_intent(&intent).await?))
}

fn proxy_ssh_intent(args: &cli::ProxyArgs) -> SshTargetIntent {
    SshTargetIntent {
        target: args.target.clone(),
        ssh_args: args.ssh_args.clone(),
        ssh_command: args.ssh_command.clone(),
        user: args.user.clone(),
        port: args.port,
        identity: args.identity.clone(),
        config: args.config.clone(),
        known_hosts: args.known_hosts.clone(),
        accept_new: args.accept_new,
        insecure_ignore_host_key: args.insecure_ignore_host_key,
        jump: args.jump.clone(),
    }
}

fn install_ssh_intent(args: &cli::InstallRemoteArgs) -> SshTargetIntent {
    SshTargetIntent {
        target: args.target.clone(),
        ssh_args: args.ssh_args.clone(),
        ssh_command: args.ssh_command.clone(),
        user: args.user.clone(),
        port: args.port,
        identity: args.identity.clone(),
        config: args.config.clone(),
        known_hosts: args.known_hosts.clone(),
        accept_new: args.accept_new,
        insecure_ignore_host_key: args.insecure_ignore_host_key,
        jump: args.jump.clone(),
    }
}

fn route_ssh_intent(args: &cli::RouteArgs) -> SshTargetIntent {
    SshTargetIntent {
        target: args.target.clone(),
        ssh_args: args.ssh_args.clone(),
        ssh_command: None,
        user: args.user.clone(),
        port: args.ssh_port,
        identity: args.identity.clone(),
        config: args.config.clone(),
        known_hosts: args.known_hosts.clone(),
        accept_new: args.accept_new,
        insecure_ignore_host_key: args.insecure_ignore_host_key,
        jump: args.jump.clone(),
    }
}
