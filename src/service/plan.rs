use std::{fs, io::Read, net::SocketAddr, path::PathBuf, process::Command};
#[cfg(windows)]
use std::{thread, time::Duration};

use anyhow::{Context, Result, bail};
use sha2::Digest;

use super::inventory::ServiceInventory;
use crate::{cli, config, control_socket, peer_lifecycle};

pub(crate) struct ServicePlan {
    pub(crate) command: cli::ServiceCommand,
    pub(crate) requested_scope: cli::ServiceScope,
    pub(crate) scope: ServiceScope,
    pub(crate) resolution: ServiceInventory,
    pub(crate) source_exe: PathBuf,
    pub(crate) exe: PathBuf,
    pub(crate) copy_exe: bool,
    pub(crate) endpoint: String,
    pub(crate) transport: Option<std::net::SocketAddr>,
    pub(crate) token: Option<String>,
    pub(crate) tls_transport: Option<SocketAddr>,
    pub(crate) quic_transport: Option<SocketAddr>,
    pub(crate) tls_cert: Option<PathBuf>,
    pub(crate) tls_key: Option<PathBuf>,
    pub(crate) tls_client_ca: Option<PathBuf>,
    pub(crate) report_to: Vec<String>,
    #[cfg(windows)]
    pub(crate) elevate: bool,
    pub(crate) config_path: PathBuf,
    pub(crate) route_store_path: PathBuf,
    pub(crate) config_to_save: Option<config::AppConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceScope {
    User,
    System,
}

impl ServicePlan {
    pub(crate) fn new(args: cli::ServiceArgs, mut config: config::AppConfig) -> Result<Self> {
        let source_exe = std::env::current_exe().context("failed to locate current executable")?;
        let should_materialize_config = matches!(
            args.command,
            cli::ServiceCommand::Ensure | cli::ServiceCommand::Install | cli::ServiceCommand::Print
        );
        let copy_exe = !args.no_copy;
        let probe_chain = super::inventory::collect_service_inventory();
        let resolution = super::inventory::resolve_service_inventory(args.scope, probe_chain);
        let scope = resolution
            .selected_scope
            .unwrap_or_else(preferred_install_scope);
        let exe = if copy_exe {
            args.install_dir
                .unwrap_or(default_install_dir(scope, &source_exe)?)
                .join(executable_name())
        } else {
            source_exe.clone()
        };
        let explicit_control = args.control.clone();
        let endpoint = explicit_control
            .clone()
            .or_else(|| config.daemon.control_endpoint.clone())
            .unwrap_or_else(|| {
                config
                    .daemon
                    .control_listen
                    .map(|addr| format!("tcp://{addr}"))
                    .unwrap_or_else(control_socket::default_endpoint_string)
            });
        let transport = if args.no_transport {
            None
        } else {
            let configured = args
                .transport
                .or(config.daemon.transport_listen)
                .or_else(|| Some(control_socket::default_user_tcp_addr(19080)));
            configured.map(|addr| {
                if addr.port() == 0
                    || (args.transport.is_none() && config.daemon.transport_listen.is_none())
                {
                    config::first_available_addr(addr, 200)
                } else {
                    addr
                }
            })
        };
        let token = match args.token {
            Some(token) => {
                if should_materialize_config {
                    let token_changed = config.daemon.token.as_deref() != Some(token.as_str());
                    config.daemon.token = Some(token.clone());
                    if token_changed || config.daemon.token_metadata.is_none() {
                        config.daemon.token_metadata =
                            Some(config::TokenMetadata::new("daemon-control-transport"));
                    }
                }
                Some(token)
            }
            None if should_materialize_config => Some(config.ensure_daemon_token()?),
            None => config.daemon.token.clone(),
        };
        if should_materialize_config {
            config.ensure_node_identity()?;
        }
        let tls_transport = args.tls_transport.or(config.daemon.tls_transport_listen);
        let quic_transport = args.quic_transport.or(config.daemon.quic_transport_listen);
        let tls_cert = args
            .tls_cert
            .or_else(|| config.daemon.tls_cert.as_ref().map(config::expand_path));
        let tls_key = args
            .tls_key
            .or_else(|| config.daemon.tls_key.as_ref().map(config::expand_path));
        let tls_client_ca = args.tls_client_ca.or_else(|| {
            config
                .daemon
                .tls_client_ca
                .as_ref()
                .map(config::expand_path)
        });
        let report_to = if args.report_to.is_empty() {
            config.daemon.report_to.clone()
        } else {
            args.report_to
        };
        if should_materialize_config {
            if explicit_control.is_some() {
                config.daemon.control_endpoint = Some(endpoint.clone());
                config.daemon.control_listen = None;
            } else if config.daemon.control_endpoint.is_none()
                && config.daemon.control_listen.is_none()
            {
                config.daemon.control_endpoint = Some(endpoint.clone());
            }
            if config.daemon.transport_listen.is_none() {
                config.daemon.transport_listen = transport;
            }
            if config.daemon.tls_transport_listen.is_none() {
                config.daemon.tls_transport_listen = tls_transport;
            }
            if config.daemon.quic_transport_listen.is_none() {
                config.daemon.quic_transport_listen = quic_transport;
            }
            if config.daemon.tls_cert.is_none() {
                config.daemon.tls_cert = tls_cert.clone();
            }
            if config.daemon.tls_key.is_none() {
                config.daemon.tls_key = tls_key.clone();
            }
            if config.daemon.tls_client_ca.is_none() {
                config.daemon.tls_client_ca = tls_client_ca.clone();
            }
            if config.daemon.report_to.is_empty() {
                config.daemon.report_to = report_to.clone();
            }
        }
        let route_store_path = config
            .daemon
            .routes_path
            .as_ref()
            .map(config::expand_path)
            .unwrap_or(config::routes_path()?);
        let config_to_save = should_materialize_config.then_some(config);
        Ok(Self {
            command: args.command,
            requested_scope: args.scope,
            scope,
            resolution,
            source_exe,
            exe,
            copy_exe,
            endpoint,
            transport,
            token,
            tls_transport,
            quic_transport,
            tls_cert,
            tls_key,
            tls_client_ca,
            report_to,
            #[cfg(windows)]
            elevate: args.elevate,
            config_path: config::config_path()?,
            route_store_path,
            config_to_save,
        })
    }

    pub(crate) fn daemon_command(&self) -> String {
        let transport = self
            .transport
            .map(|addr| format!(" --transport {addr}"))
            .unwrap_or_default();
        let token = self
            .token
            .as_ref()
            .map(|token| format!(" --token {}", command_quote(token)))
            .unwrap_or_default();
        let tls_transport = self
            .tls_transport
            .map(|addr| format!(" --tls-transport {addr}"))
            .unwrap_or_default();
        let quic_transport = self
            .quic_transport
            .map(|addr| format!(" --quic-transport {addr}"))
            .unwrap_or_default();
        let tls_cert = self
            .tls_cert
            .as_ref()
            .map(|path| format!(" --tls-cert {}", command_quote(&path.display().to_string())))
            .unwrap_or_default();
        let tls_key = self
            .tls_key
            .as_ref()
            .map(|path| format!(" --tls-key {}", command_quote(&path.display().to_string())))
            .unwrap_or_default();
        let tls_client_ca = self
            .tls_client_ca
            .as_ref()
            .map(|path| {
                format!(
                    " --tls-client-ca {}",
                    command_quote(&path.display().to_string())
                )
            })
            .unwrap_or_default();
        let report_to = self
            .report_to
            .iter()
            .map(|endpoint| format!(" --report-to {}", command_quote(endpoint)))
            .collect::<String>();
        format!(
            "{} daemon serve --control {}{}{}{}{}{}{}{}{}",
            command_quote(&self.exe.display().to_string()),
            command_quote(&self.endpoint),
            transport,
            token,
            tls_transport,
            quic_transport,
            tls_cert,
            tls_key,
            tls_client_ca,
            report_to
        )
    }

    pub(crate) fn install_binary(&self) -> Result<()> {
        if !self.copy_exe {
            return Ok(());
        }
        if self.source_exe == self.exe {
            return Ok(());
        }
        if let Some(parent) = self.exe.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        copy_binary(&self.source_exe, &self.exe)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&self.exe)?.permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&self.exe, permissions)?;
        }
        Ok(())
    }

    pub(crate) fn lifecycle_spec(&self) -> peer_lifecycle::spec::PeerLifecycleSpec {
        let provider = lifecycle_provider_for_scope(self.scope);
        let state_dir = self
            .config_path
            .parent()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| ".".to_string());
        peer_lifecycle::spec::PeerLifecycleSpec::local_daemon(
            "local",
            self.exe.display().to_string(),
            provider,
            platform_service_name(self.scope),
            Some(self.endpoint.clone()),
            self.transport,
            self.token.clone(),
            state_dir,
        )
    }
}

pub(crate) fn lifecycle_provider_for_scope(
    scope: ServiceScope,
) -> peer_lifecycle::service_provider::ServiceProviderKind {
    if cfg!(windows) {
        match scope {
            ServiceScope::System => {
                peer_lifecycle::service_provider::ServiceProviderKind::WindowsScmSystem
            }
            ServiceScope::User => {
                peer_lifecycle::service_provider::ServiceProviderKind::WindowsScheduledTaskUser
            }
        }
    } else if cfg!(target_os = "macos") {
        match scope {
            ServiceScope::System => {
                peer_lifecycle::service_provider::ServiceProviderKind::LaunchdSystem
            }
            ServiceScope::User => {
                peer_lifecycle::service_provider::ServiceProviderKind::LaunchdUser
            }
        }
    } else {
        match scope {
            ServiceScope::System => {
                peer_lifecycle::service_provider::ServiceProviderKind::SystemdSystem
            }
            ServiceScope::User => {
                peer_lifecycle::service_provider::ServiceProviderKind::SystemdUser
            }
        }
    }
}

fn copy_binary(source: &PathBuf, target: &PathBuf) -> Result<()> {
    #[cfg(windows)]
    {
        let mut last_error = None;
        for _ in 0..20 {
            match fs::copy(source, target) {
                Ok(_) => return Ok(()),
                Err(err) => {
                    last_error = Some(err);
                    thread::sleep(Duration::from_millis(250));
                }
            }
        }
        let err = last_error.expect("copy loop should record an error");
        return Err(err).with_context(|| {
            format!(
                "failed to copy {} to {}",
                source.display(),
                target.display()
            )
        });
    }
    #[cfg(not(windows))]
    {
        fs::copy(source, target).with_context(|| {
            format!(
                "failed to copy {} to {}",
                source.display(),
                target.display()
            )
        })?;
        Ok(())
    }
}

fn default_install_dir(scope: ServiceScope, source_exe: &PathBuf) -> Result<PathBuf> {
    #[cfg(windows)]
    if matches!(scope, ServiceScope::System) {
        let root = std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
        let hash = short_file_hash(source_exe).unwrap_or_else(|_| "unknown".to_string());
        return Ok(root
            .join("ssh_proxy")
            .join("bin")
            .join(format!("{}-{hash}", env!("CARGO_PKG_VERSION"))));
    }
    default_local_install_dir()
}

fn default_local_install_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        Ok(dirs::data_local_dir()
            .context("cannot determine LOCALAPPDATA")?
            .join("ssh_proxy")
            .join("bin"))
    }
    #[cfg(not(windows))]
    {
        Ok(dirs::home_dir()
            .context("cannot determine home directory")?
            .join(".local")
            .join("bin"))
    }
}

fn short_file_hash(path: &PathBuf) -> Result<String> {
    let mut file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if read == 0 {
            break;
        }
        sha2::Digest::update(&mut hasher, &buffer[..read]);
    }
    let hash = sha2::Digest::finalize(hasher)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(hash.chars().take(12).collect())
}

fn executable_name() -> &'static str {
    if cfg!(windows) {
        "ssh_proxy.exe"
    } else {
        "ssh_proxy"
    }
}

pub(crate) fn preferred_install_scope() -> ServiceScope {
    if is_admin() {
        ServiceScope::System
    } else {
        ServiceScope::User
    }
}

pub(crate) fn command_quote(value: &str) -> String {
    if cfg!(windows) {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        sh_quote(value)
    }
}

pub(crate) fn platform_service_name(scope: ServiceScope) -> String {
    match scope {
        ServiceScope::System => "ssh_proxy".to_string(),
        ServiceScope::User => {
            #[cfg(windows)]
            {
                let user = current_windows_user_component();
                format!("ssh_proxy-{user}")
            }
            #[cfg(not(windows))]
            {
                "ssh_proxy".to_string()
            }
        }
    }
}

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(crate) fn ensure_admin(message: &str) -> Result<()> {
    if is_admin() {
        Ok(())
    } else {
        bail!("{message}; rerun as administrator/root or use --scope user")
    }
}

#[cfg(unix)]
pub(crate) fn is_admin() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .is_some_and(|uid| uid.trim() == "0")
}

#[cfg(windows)]
pub(crate) fn is_admin() -> bool {
    Command::new("net")
        .arg("session")
        .output()
        .is_ok_and(|out| out.status.success())
}

#[cfg(windows)]
fn current_windows_user_component() -> String {
    let raw = whoami::username().unwrap_or_else(|_| "user".to_string());
    let filtered: String = raw
        .chars()
        .map(|ch| match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' => ch,
            _ => '-',
        })
        .collect();
    let trimmed = filtered.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "user".to_string()
    } else {
        trimmed
    }
}
