use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::{
    cli,
    config::{self, AppConfig},
    node_daemon::ProxySessionSpec,
};

use super::service_provider::ServiceProviderKind;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PeerSshSpec {
    pub(crate) ssh_args: Vec<String>,
    pub(crate) ssh_command: Option<String>,
    pub(crate) user: Option<String>,
    pub(crate) port: Option<u16>,
    pub(crate) identity: Vec<PathBuf>,
    pub(crate) config: Option<PathBuf>,
    pub(crate) known_hosts: Option<PathBuf>,
    pub(crate) accept_new: bool,
    pub(crate) insecure_ignore_host_key: bool,
    pub(crate) jump: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct PeerBootstrapSpec {
    pub(crate) target: String,
    pub(crate) alias: Option<String>,
    pub(crate) force: bool,
    pub(crate) ssh: PeerSshSpec,
    pub(crate) remote_path: Option<String>,
    pub(crate) remote_bin: Option<PathBuf>,
    pub(crate) remote_os: cli::RemoteOs,
    pub(crate) remote_token: Option<String>,
    pub(crate) remote_tcp: SocketAddr,
    pub(crate) remote_control: SocketAddr,
    pub(crate) local_node_id: Option<String>,
    pub(crate) local_node_name: Option<String>,
    pub(crate) local_control_endpoint: Option<String>,
    pub(crate) local_transport: Option<SocketAddr>,
    pub(crate) remote_node_id: Option<String>,
    pub(crate) remote_node_name: Option<String>,
    pub(crate) remote_tls_transport: Option<SocketAddr>,
    pub(crate) remote_quic_transport: Option<SocketAddr>,
    pub(crate) remote_tls_cert: Option<String>,
    pub(crate) remote_tls_key: Option<String>,
    pub(crate) remote_tls_client_ca: Option<String>,
    pub(crate) persist: cli::PersistMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PeerLifecycleRole {
    LocalDaemon,
    RemotePeer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PeerLifecyclePlatform {
    Windows,
    Linux,
    Macos,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PeerLifecycleScope {
    User,
    System,
    Managed,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RollbackPolicy {
    None,
    PreserveExisting,
    StopAndRestore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PeerLifecycleSpec {
    pub(crate) role: PeerLifecycleRole,
    pub(crate) target: String,
    pub(crate) platform: PeerLifecyclePlatform,
    pub(crate) scope: PeerLifecycleScope,
    pub(crate) provider: ServiceProviderKind,
    pub(crate) service_name: String,
    pub(crate) binary_path: String,
    pub(crate) transport: Option<SocketAddr>,
    pub(crate) control_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) token: Option<String>,
    pub(crate) state_dir: String,
    pub(crate) rollback_policy: RollbackPolicy,
}

impl PeerLifecycleSpec {
    pub(crate) fn remote_peer(
        target: impl Into<String>,
        remote_path: impl Into<String>,
        args: &cli::InstallRemoteArgs,
        provider: ServiceProviderKind,
    ) -> Self {
        Self {
            role: PeerLifecycleRole::RemotePeer,
            target: target.into(),
            platform: platform_from_remote_os(args.remote_os),
            scope: scope_from_persist(args.persist),
            provider,
            service_name: "ssh-proxy-helper".to_string(),
            binary_path: remote_path.into(),
            transport: Some(args.remote_tcp),
            control_endpoint: Some(format!("tcp://{}", args.remote_control)),
            token: args.remote_token.clone(),
            state_dir: remote_state_dir(args.remote_os),
            rollback_policy: RollbackPolicy::PreserveExisting,
        }
    }

    pub(crate) fn local_daemon(
        target: impl Into<String>,
        binary_path: impl Into<String>,
        provider: ServiceProviderKind,
        service_name: impl Into<String>,
        control_endpoint: Option<String>,
        transport: Option<SocketAddr>,
        token: Option<String>,
        state_dir: impl Into<String>,
    ) -> Self {
        Self {
            role: PeerLifecycleRole::LocalDaemon,
            target: target.into(),
            platform: local_platform(),
            scope: if provider.requires_elevation() {
                PeerLifecycleScope::System
            } else {
                PeerLifecycleScope::User
            },
            provider,
            service_name: service_name.into(),
            binary_path: binary_path.into(),
            transport,
            control_endpoint,
            token,
            state_dir: state_dir.into(),
            rollback_policy: RollbackPolicy::StopAndRestore,
        }
    }
}

impl PeerBootstrapSpec {
    pub(crate) fn alias_or_target(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.target)
    }

    pub(crate) fn into_install_args(self) -> cli::InstallRemoteArgs {
        cli::InstallRemoteArgs {
            target: self.target,
            ssh_args: self.ssh.ssh_args,
            ssh_command: self.ssh.ssh_command,
            user: self.ssh.user,
            port: self.ssh.port,
            identity: self.ssh.identity,
            config: self.ssh.config,
            known_hosts: self.ssh.known_hosts,
            accept_new: self.ssh.accept_new,
            insecure_ignore_host_key: self.ssh.insecure_ignore_host_key,
            jump: self.ssh.jump,
            remote_path: self.remote_path,
            remote_bin: self.remote_bin,
            remote_os: self.remote_os,
            remote_token: self.remote_token,
            remote_tcp: self.remote_tcp,
            remote_control: self.remote_control,
            local_node_id: self.local_node_id,
            local_node_name: self.local_node_name,
            local_control_endpoint: self.local_control_endpoint,
            local_transport: self.local_transport,
            remote_node_id: self.remote_node_id,
            remote_node_name: self.remote_node_name,
            remote_tls_transport: self.remote_tls_transport,
            remote_quic_transport: self.remote_quic_transport,
            remote_tls_cert: self.remote_tls_cert,
            remote_tls_key: self.remote_tls_key,
            remote_tls_client_ca: self.remote_tls_client_ca,
            persist: self.persist,
        }
    }
}

pub(crate) fn bootstrap_spec_from_proxy_session(
    config: Option<&AppConfig>,
    spec: &ProxySessionSpec,
) -> Result<PeerBootstrapSpec> {
    let ssh = spec
        .ssh
        .as_ref()
        .map(|ssh| PeerSshSpec {
            ssh_args: ssh.ssh_args(),
            ssh_command: None,
            user: ssh.user.clone(),
            port: ssh.port,
            identity: ssh.identity.clone(),
            config: ssh.config.clone(),
            known_hosts: ssh.known_hosts.clone(),
            accept_new: ssh.accept_new,
            insecure_ignore_host_key: false,
            jump: ssh.jump.clone(),
        })
        .unwrap_or_default();
    let mut bootstrap = PeerBootstrapSpec {
        target: spec.target.clone(),
        alias: Some(spec.target.clone()),
        force: false,
        ssh,
        remote_path: None,
        remote_bin: None,
        remote_os: cli::RemoteOs::Auto,
        remote_token: None,
        remote_tcp: default_remote_tcp()?,
        remote_control: default_remote_control()?,
        local_node_id: None,
        local_node_name: None,
        local_control_endpoint: None,
        local_transport: None,
        remote_node_id: None,
        remote_node_name: None,
        remote_tls_transport: None,
        remote_quic_transport: None,
        remote_tls_cert: None,
        remote_tls_key: None,
        remote_tls_client_ca: None,
        persist: cli::PersistMode::Auto,
    };
    if let Some(config) = config {
        apply_local_daemon_defaults(&mut bootstrap, config);
    }
    Ok(bootstrap)
}

pub(crate) fn bootstrap_spec_from_peer_bootstrap(
    args: cli::PeerBootstrapArgs,
) -> PeerBootstrapSpec {
    PeerBootstrapSpec {
        target: args.target,
        alias: args.alias,
        force: args.force,
        ssh: PeerSshSpec {
            ssh_args: args.ssh_args,
            ssh_command: None,
            user: args.user,
            port: args.port,
            identity: args.identity,
            config: args.config,
            known_hosts: args.known_hosts,
            accept_new: args.accept_new,
            insecure_ignore_host_key: args.insecure_ignore_host_key,
            jump: args.jump,
        },
        remote_path: args.remote_path,
        remote_bin: args.remote_bin,
        remote_os: args.remote_os,
        remote_token: args.remote_token,
        remote_tcp: args.remote_tcp,
        remote_control: args.remote_control,
        local_node_id: None,
        local_node_name: None,
        local_control_endpoint: None,
        local_transport: None,
        remote_node_id: None,
        remote_node_name: None,
        remote_tls_transport: None,
        remote_quic_transport: None,
        remote_tls_cert: None,
        remote_tls_key: None,
        remote_tls_client_ca: None,
        persist: cli::PersistMode::Auto,
    }
}

pub(crate) fn install_args_from_proxy_session(
    config: Option<&AppConfig>,
    spec: &ProxySessionSpec,
) -> Result<cli::InstallRemoteArgs> {
    Ok(bootstrap_spec_from_proxy_session(config, spec)?.into_install_args())
}

pub(crate) fn install_args_from_peer_bootstrap(
    args: cli::PeerBootstrapArgs,
) -> cli::InstallRemoteArgs {
    bootstrap_spec_from_peer_bootstrap(args).into_install_args()
}

fn apply_local_daemon_defaults(bootstrap: &mut PeerBootstrapSpec, config: &AppConfig) {
    bootstrap.remote_token = bootstrap
        .remote_token
        .clone()
        .or_else(|| config.daemon.token.clone());
    bootstrap.local_node_id = config.identity.node_id.clone();
    bootstrap.local_node_name = config.identity.node_name.clone();
    bootstrap.local_control_endpoint = config.daemon.control_endpoint.clone();
    bootstrap.local_transport = config.daemon.transport_listen;
    bootstrap.remote_tls_transport = config.daemon.tls_transport_listen;
    bootstrap.remote_quic_transport = config.daemon.quic_transport_listen;
    bootstrap.remote_tls_cert = config
        .daemon
        .tls_cert
        .as_ref()
        .map(|path| path.display().to_string());
    bootstrap.remote_tls_key = config
        .daemon
        .tls_key
        .as_ref()
        .map(|path| path.display().to_string());
    bootstrap.remote_tls_client_ca = config
        .daemon
        .tls_client_ca
        .as_ref()
        .map(|path| path.display().to_string());
}

fn default_remote_tcp() -> Result<SocketAddr> {
    "127.0.0.1:19080"
        .parse()
        .map_err(|err| anyhow!("invalid default remote tcp: {err}"))
}

fn default_remote_control() -> Result<SocketAddr> {
    "127.0.0.1:19081"
        .parse()
        .map_err(|err| anyhow!("invalid default remote control: {err}"))
}

pub(crate) fn generated_remote_node_id() -> Result<String> {
    Ok(format!("spx-{}", config::generate_token()?))
}

fn platform_from_remote_os(remote_os: cli::RemoteOs) -> PeerLifecyclePlatform {
    match remote_os {
        cli::RemoteOs::Windows => PeerLifecyclePlatform::Windows,
        cli::RemoteOs::Unix | cli::RemoteOs::Auto => PeerLifecyclePlatform::Linux,
    }
}

fn scope_from_persist(persist: cli::PersistMode) -> PeerLifecycleScope {
    match persist {
        cli::PersistMode::None => PeerLifecycleScope::None,
        cli::PersistMode::Nohup => PeerLifecycleScope::Managed,
        cli::PersistMode::Systemd
        | cli::PersistMode::Launchd
        | cli::PersistMode::Schtasks
        | cli::PersistMode::Auto => PeerLifecycleScope::User,
    }
}

fn remote_state_dir(remote_os: cli::RemoteOs) -> String {
    match remote_os {
        cli::RemoteOs::Windows => r"%USERPROFILE%\.ssh_proxy".to_string(),
        cli::RemoteOs::Unix | cli::RemoteOs::Auto => "$HOME/.ssh_proxy".to_string(),
    }
}

fn local_platform() -> PeerLifecyclePlatform {
    if cfg!(windows) {
        PeerLifecyclePlatform::Windows
    } else if cfg!(target_os = "macos") {
        PeerLifecyclePlatform::Macos
    } else if cfg!(target_os = "linux") {
        PeerLifecyclePlatform::Linux
    } else {
        PeerLifecyclePlatform::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_session_builder_preserves_vscode_ssh_fields() {
        let spec = ProxySessionSpec {
            target: "office".to_string(),
            workspace_id: Some("me@host".to_string()),
            ssh: Some(crate::node_daemon::SshTargetSpec {
                host_name: Some("10.0.0.2".to_string()),
                user: Some("me".to_string()),
                port: Some(2200),
                identity: vec![PathBuf::from("id_ed25519")],
                config: Some(PathBuf::from("config")),
                known_hosts: Some(PathBuf::from("known_hosts")),
                jump: vec!["bastion".to_string()],
                accept_new: true,
            }),
            workspace_paths: Vec::new(),
            local_proxy: "http://127.0.0.1:10808/".to_string(),
            remote_bind: "127.0.0.1".parse().unwrap(),
            remote_port_policy: crate::node_daemon::RemotePortPolicy {
                preferred: 17890,
                auto_pick: true,
            },
            connect_mode: cli::RouteConnectMode::Auto,
            apply_policy: Default::default(),
        };

        let args = install_args_from_proxy_session(None, &spec).unwrap();

        assert_eq!(args.target, "office");
        assert_eq!(args.ssh_args, vec!["-o", "HostName=10.0.0.2"]);
        assert_eq!(args.user.as_deref(), Some("me"));
        assert_eq!(args.port, Some(2200));
        assert_eq!(args.identity, vec![PathBuf::from("id_ed25519")]);
        assert_eq!(args.config, Some(PathBuf::from("config")));
        assert_eq!(args.known_hosts, Some(PathBuf::from("known_hosts")));
        assert_eq!(args.jump, vec!["bastion"]);
        assert!(args.accept_new);
        assert_eq!(args.persist, cli::PersistMode::Auto);
    }

    #[test]
    fn lifecycle_spec_models_remote_peer_without_shell_policy() {
        let mut args = install_args_from_peer_bootstrap(crate::cli::PeerBootstrapArgs {
            target: "edge".to_string(),
            alias: None,
            force: false,
            ssh_args: Vec::new(),
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
            remote_os: cli::RemoteOs::Unix,
            remote_token: Some("secret".to_string()),
            remote_tcp: "127.0.0.1:19080".parse().unwrap(),
            remote_control: "127.0.0.1:19081".parse().unwrap(),
        });
        args.persist = cli::PersistMode::Systemd;
        let spec = PeerLifecycleSpec::remote_peer(
            "edge",
            "/home/me/.local/bin/ssh_proxy",
            &args,
            ServiceProviderKind::SystemdUser,
        );

        assert_eq!(spec.role, PeerLifecycleRole::RemotePeer);
        assert_eq!(spec.platform, PeerLifecyclePlatform::Linux);
        assert_eq!(spec.scope, PeerLifecycleScope::User);
        assert_eq!(
            spec.control_endpoint.as_deref(),
            Some("tcp://127.0.0.1:19081")
        );
    }
}
