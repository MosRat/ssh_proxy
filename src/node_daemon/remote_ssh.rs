use anyhow::{Result, anyhow};

use crate::{cli, config};

use super::proxy_session::ProxySessionSpec;

pub(super) fn install_args_from_spec(
    config: &config::AppConfig,
    spec: &ProxySessionSpec,
) -> Result<cli::InstallRemoteArgs> {
    let mut args = cli::InstallRemoteArgs {
        target: spec.target.clone(),
        ssh_args: Vec::new(),
        ssh_command: None,
        user: None,
        port: None,
        identity: Vec::new(),
        config: None,
        known_hosts: None,
        accept_new: false,
        insecure_ignore_host_key: false,
        jump: Vec::new(),
        remote_path: None,
        remote_bin: None,
        remote_os: cli::RemoteOs::Auto,
        remote_token: config.daemon.token.clone(),
        remote_tcp: "127.0.0.1:19080"
            .parse()
            .map_err(|err| anyhow!("invalid default remote tcp: {err}"))?,
        remote_control: "127.0.0.1:19081"
            .parse()
            .map_err(|err| anyhow!("invalid default remote control: {err}"))?,
        local_node_id: config.identity.node_id.clone(),
        local_node_name: config.identity.node_name.clone(),
        local_control_endpoint: config.daemon.control_endpoint.clone(),
        local_transport: config.daemon.transport_listen,
        remote_node_id: None,
        remote_node_name: None,
        remote_tls_transport: config.daemon.tls_transport_listen,
        remote_quic_transport: config.daemon.quic_transport_listen,
        remote_tls_cert: config
            .daemon
            .tls_cert
            .as_ref()
            .map(|path| path.display().to_string()),
        remote_tls_key: config
            .daemon
            .tls_key
            .as_ref()
            .map(|path| path.display().to_string()),
        remote_tls_client_ca: config
            .daemon
            .tls_client_ca
            .as_ref()
            .map(|path| path.display().to_string()),
        persist: cli::PersistMode::Auto,
    };
    config.apply_install_defaults(&mut args, Some(&spec.target))?;
    if let Some(profile) = config.profiles.get(&spec.target) {
        args.target = profile
            .target
            .clone()
            .unwrap_or_else(|| spec.target.clone());
    }
    if let Some(ssh) = spec.ssh.as_ref() {
        args.ssh_args = ssh.ssh_args();
        args.user = ssh.user.clone();
        args.port = ssh.port;
        args.identity = ssh.identity.clone();
        args.config = ssh.config.clone();
        args.known_hosts = ssh.known_hosts.clone();
        args.jump = ssh.jump.clone();
        args.accept_new = ssh.accept_new;
    }
    Ok(args)
}
