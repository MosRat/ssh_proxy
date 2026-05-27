use std::{
    net::SocketAddr,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};

use crate::{cli, control_socket};

mod certs;
mod diagnostics;
mod io;
pub use io::{
    certs_dir, config_path, daemon_state_path, file_sha256_fingerprint, jobs_path, peers_path,
    routes_path, save_text_file_private, sessions_path,
};
mod peer;
mod profile;
pub use profile::expand_path;
mod schema;
#[cfg(test)]
pub use schema::DaemonConfig;
pub use schema::{
    AppConfig, CONFIG_SCHEMA_VERSION, NodeIdentity, PeerRecord, ProxyProfile, TokenMetadata,
};

impl AppConfig {
    pub fn load_default() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let config: Self =
            toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))?;
        config.validate_schema()?;
        Ok(config)
    }

    pub fn save_default(&self) -> Result<()> {
        self.validate_schema()?;
        let path = config_path()?;
        let text = toml::to_string_pretty(self).context("failed to encode config TOML")?;
        save_text_file_private(&path, &text)
    }

    pub fn validate_schema(&self) -> Result<()> {
        if self.schema_version > CONFIG_SCHEMA_VERSION {
            bail!(
                "config schema_version {} is newer than this binary supports ({CONFIG_SCHEMA_VERSION}); upgrade ssh_proxy",
                self.schema_version
            );
        }
        Ok(())
    }

    pub fn ensure_daemon_token(&mut self) -> Result<String> {
        if let Some(token) = &self.daemon.token {
            if self.daemon.token_metadata.is_none() {
                self.daemon.token_metadata = Some(TokenMetadata::new("daemon-control-transport"));
            }
            return Ok(token.clone());
        }
        let token = generate_token()?;
        self.daemon.token = Some(token.clone());
        self.daemon.token_metadata = Some(TokenMetadata::new("daemon-control-transport"));
        Ok(token)
    }

    pub fn rotate_daemon_token(&mut self) -> Result<String> {
        let token = generate_token()?;
        let generation = self
            .daemon
            .token_metadata
            .as_ref()
            .map(|metadata| metadata.generation.saturating_add(1))
            .unwrap_or(1);
        self.daemon.token = Some(token.clone());
        self.daemon.token_metadata = Some(TokenMetadata::rotated(
            "daemon-control-transport",
            generation,
        ));
        Ok(token)
    }

    pub fn ensure_node_identity(&mut self) -> Result<NodeIdentity> {
        if self.identity.node_id.is_none() {
            self.identity.node_id = Some(format!("spx-{}", generate_token()?));
        }
        if self.identity.node_name.is_none() {
            self.identity.node_name = Some(default_node_name());
        }
        if self.identity.secret.is_none() {
            self.identity.secret = Some(generate_token()?);
        }
        Ok(self.identity.clone())
    }

    pub fn proxy_from_profile(&self, name: &str) -> Result<cli::ProxyArgs> {
        let profile = self.profiles.get(name).ok_or_else(|| {
            let path = config_path()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| "~/.ssh_proxy/config.toml".to_string());
            anyhow!("profile {name:?} not found in {path}")
        })?;
        let target = profile.target.clone().unwrap_or_else(|| name.to_string());
        let mut args = profile::default_proxy_args(target);
        self.apply_proxy_defaults(&mut args, Some(name))?;
        Ok(args)
    }

    pub fn apply_proxy_defaults(
        &self,
        args: &mut cli::ProxyArgs,
        profile_name: Option<&str>,
    ) -> Result<()> {
        profile::apply_profile(args, &self.defaults, "defaults")?;
        if let Some(profile) = profile_name
            .and_then(|name| self.profiles.get(name))
            .or_else(|| self.profiles.get(&args.target))
        {
            if let Some(target) = &profile.target {
                if profile_name.is_some() {
                    args.target = target.clone();
                }
            }
            profile::apply_profile(args, profile, "profile")?;
        }
        if args.control_listen.is_none() {
            args.control_listen = self.daemon.control_listen;
        }
        Ok(())
    }

    pub fn apply_install_defaults(
        &self,
        args: &mut cli::InstallRemoteArgs,
        profile_name: Option<&str>,
    ) -> Result<()> {
        let mut proxy = profile::default_proxy_args(args.target.clone());
        self.apply_proxy_defaults(&mut proxy, profile_name.or(Some(&args.target)))?;
        if profile_name.is_some() || self.profiles.contains_key(&args.target) {
            args.target = proxy.target;
        }
        if args.ssh_args.is_empty() {
            args.ssh_args = proxy.ssh_args;
        }
        args.user = args.user.take().or(proxy.user);
        args.port = args.port.or(proxy.port);
        if args.identity.is_empty() {
            args.identity = proxy.identity;
        }
        args.config = args.config.take().or(proxy.config);
        args.known_hosts = args.known_hosts.take().or(proxy.known_hosts);
        args.accept_new |= proxy.accept_new;
        args.insecure_ignore_host_key |= proxy.insecure_ignore_host_key;
        if args.jump.is_empty() {
            args.jump = proxy.jump;
        }
        args.remote_path = args.remote_path.take().or(proxy.remote_path);
        args.remote_bin = args.remote_bin.take().or(proxy.remote_bin);
        if args.remote_os == cli::RemoteOs::Auto {
            args.remote_os = proxy.remote_os;
        }
        args.remote_token = args.remote_token.take().or(proxy.remote_token);
        if args.remote_tcp == SocketAddr::from(([127, 0, 0, 1], 19080)) {
            args.remote_tcp = proxy.remote_tcp;
        }
        if args.remote_control == SocketAddr::from(([127, 0, 0, 1], 19081)) {
            args.remote_control = proxy.remote_control;
        }
        Ok(())
    }
}

pub async fn run(args: cli::ConfigArgs) -> Result<()> {
    match args.command {
        cli::ConfigCommand::Path => {
            println!("{}", config_path()?.display());
        }
        cli::ConfigCommand::Sample => {
            println!("{}", sample_config());
        }
        cli::ConfigCommand::Init { force } => {
            let path = config_path()?;
            if path.exists() && !force {
                bail!(
                    "{} already exists; pass --force to overwrite",
                    path.display()
                );
            }
            let mut config = AppConfig::default();
            config.ensure_node_identity()?;
            config.daemon.control_endpoint = Some(control_socket::default_endpoint_string());
            config.daemon.transport_listen = Some(control_socket::default_user_tcp_addr(19080));
            config.ensure_daemon_token()?;
            config.save_default()?;
            println!("initialized {}", path.display());
        }
        cli::ConfigCommand::Show => {
            let config = AppConfig::load_default()?;
            println!("{}", toml::to_string_pretty(&config)?);
        }
        cli::ConfigCommand::Inspect => {
            let config = AppConfig::load_default()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&diagnostics::inspect(&config))?
            );
        }
        cli::ConfigCommand::ExportDescriptor => {
            let mut config = AppConfig::load_default()?;
            config.ensure_node_identity()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&diagnostics::export_descriptor(&config))?
            );
        }
        cli::ConfigCommand::ImportDescriptor(import_args) => {
            let mut config = AppConfig::load_default()?;
            diagnostics::import_peer_descriptor(&mut config, &import_args)?;
            config.save_default()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "ok": true,
                    "alias": import_args.alias,
                    "changed": true,
                    "trust": import_args.trust,
                }))?
            );
        }
        cli::ConfigCommand::Profiles => {
            let config = AppConfig::load_default()?;
            for (name, profile) in profile::sorted_profiles(&config) {
                let target = profile.target.as_deref().unwrap_or(name);
                let listen = profile
                    .listen
                    .map(|addr| addr.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let transport = profile.remote_transport.as_deref().unwrap_or("auto");
                println!("{name}\ttarget={target}\tlisten={listen}\tremote_transport={transport}");
            }
        }
        cli::ConfigCommand::Peers => {
            let config = AppConfig::load_default()?;
            for (name, peer) in peer::sorted_peers(&config) {
                let node = peer
                    .node_name
                    .as_deref()
                    .or(peer.node_id.as_deref())
                    .unwrap_or("-");
                let control = peer.control_endpoint.as_deref().unwrap_or("-");
                let transport = peer
                    .transport
                    .map(|addr| addr.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let trust = peer.trust.as_deref().unwrap_or("-");
                let token = if peer.token.is_some() { "yes" } else { "no" };
                let token_scope = peer
                    .token_metadata
                    .as_ref()
                    .map(|metadata| metadata.scope.as_str())
                    .unwrap_or("-");
                let protocols = peer.known_transport_protocols().join(",");
                let protocols = if protocols.is_empty() {
                    "-".to_string()
                } else {
                    protocols
                };
                println!(
                    "{name}\tnode={node}\tcontrol={control}\ttransport={transport}\tprotocols={protocols}\ttoken={token}\ttoken_scope={token_scope}\ttrust={trust}"
                );
            }
        }
        cli::ConfigCommand::ProfileSet(profile_args) => {
            let mut config = AppConfig::load_default()?;
            let name = profile_args.name.clone();
            profile::apply_profile_set(&mut config, profile_args)?;
            config.save_default()?;
            println!("saved profile {name:?} in {}", config_path()?.display());
        }
        cli::ConfigCommand::ProfileRemove { name } => {
            let mut config = AppConfig::load_default()?;
            if config.profiles.remove(&name).is_none() {
                bail!("profile {name:?} does not exist");
            }
            config.save_default()?;
            println!("removed profile {name:?}");
        }
        cli::ConfigCommand::Token { rotate } => {
            let mut config = AppConfig::load_default()?;
            let changed = if rotate {
                config.rotate_daemon_token()?;
                true
            } else if config.daemon.token.is_none() {
                config.ensure_daemon_token()?;
                true
            } else if config.daemon.token_metadata.is_none() {
                config.ensure_daemon_token()?;
                true
            } else {
                false
            };
            if changed {
                config.save_default()?;
            }
            println!(
                "daemon token: {}",
                config.daemon.token.as_deref().unwrap_or_default()
            );
            println!(
                "token metadata: {}",
                serde_json::to_string(&config.daemon.token_metadata)?
            );
        }
        cli::ConfigCommand::CertImport(cert_args) => {
            let mut config = AppConfig::load_default()?;
            let imported = certs::import(&mut config, cert_args)?;
            config.save_default()?;
            println!("{}", serde_json::to_string_pretty(&imported)?);
        }
    }
    Ok(())
}

fn sample_config() -> &'static str {
    r#"# ~/.ssh_proxy/config.toml
schema_version = 1

[daemon]
control_listen = "127.0.0.1:1081"
# The service installer enables a user-scoped transport by default.
# Set transport_listen to choose a stable port, or pass service --no-transport.
transport_listen = "127.0.0.1:19080"
# Optional alternatives:
# control_endpoint = "tcp://127.0.0.1:1081"
# control_endpoint = "unix:///run/user/1000/ssh_proxy.sock"
# control_endpoint = "npipe://ssh_proxy/control"
# tls_transport_listen = "0.0.0.0:19082"
# quic_transport_listen = "0.0.0.0:19083"
# quic_max_bidi_streams = 256
# quic_stream_receive_window = 2097152
# quic_receive_window = 16777216
# quic_keep_alive_interval_secs = 10
# quic_idle_timeout_secs = 60
# tls_cert = "~/.ssh_proxy/certs/node.pem"
# tls_key = "~/.ssh_proxy/certs/node-key.pem"
# tls_client_ca = "~/.ssh_proxy/certs/client-ca.pem"
# token = "change-me"
# report_to = ["tcp://127.0.0.1:19091"]
# routes_path = "~/.ssh_proxy/routes.json"
# route_autostart = true

# [daemon.token_metadata]
# created_at_unix = 1710000000
# rotated_at_unix = 1710000000
# scope = "daemon-control-transport"
# expires_at_unix = 0

[identity]
node_id = "spx-generated"
node_name = "user@host"
# secret is generated automatically by `config init` or `service install`.
# cert/key/ca are optional direct TLS/QUIC identity material.
# secret = "change-me"
# cert = "~/.ssh_proxy/identity/node.pem"
# key = "~/.ssh_proxy/identity/node-key.pem"
# ca = "~/.ssh_proxy/identity/ca.pem"

[defaults]
listen = "127.0.0.1:1080"
accept_new = false
deploy = "auto"
remote_os = "auto"
remote_transport = "auto"
remote_tcp = "127.0.0.1:19080"
remote_control = "127.0.0.1:19081"
# remote_quic = "198.51.100.10:19083"
# remote_tls = "198.51.100.10:19082"
# quic_max_bidi_streams = 256
# quic_stream_receive_window = 2097152
# quic_receive_window = 16777216
# quic_keep_alive_interval_secs = 10
# quic_idle_timeout_secs = 60
# remote_ca = "~/.ssh_proxy/peer-ca.pem"
# remote_name = "remote-node"
# remote_client_cert = "~/.ssh_proxy/client.pem"
# remote_client_key = "~/.ssh_proxy/client-key.pem"
# allow_plain_tcp = false
reconnect_delay_secs = 5
reconnect_max_delay_secs = 60
connect_timeout_secs = 30
transport_pool_size = 1
# ssh_session_pool_size = 2

[profiles.office]
target = "app.internal"
user = "ubuntu"
port = 22
listen = "127.0.0.1:1088"
identity = ["~/.ssh/id_ed25519"]
jump = ["bastion.example.com"]
known_hosts = "~/.ssh/known_hosts"

[profiles.persistent]
target = "app.internal"
listen = "127.0.0.1:1089"
remote_transport = "tcp"
remote_tcp = "127.0.0.1:19080"
remote_token = "change-me"

[peers.office]
node_id = "spx-remote-node-id"
node_name = "ubuntu@app.internal"
target = "app.internal"
trust = "ssh-bootstrap"
remote_path = "~/.local/bin/ssh_proxy"
control_endpoint = "tcp://127.0.0.1:19081"
transport = "127.0.0.1:19080"
token = "change-me"
token_metadata = { created_at_unix = 1710000000, scope = "peer-control-transport" }
"#
}

pub fn generate_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).context("failed to generate secure transport token")?;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("hex write to String cannot fail");
    }
    Ok(out)
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn default_node_name() -> String {
    let user = whoami::username().unwrap_or_else(|_| "unknown".to_string());
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string());
    format!("{user}@{host}")
}

pub fn is_addr_available(addr: SocketAddr) -> bool {
    std::net::TcpListener::bind(addr).is_ok()
}

pub fn first_available_addr(preferred: SocketAddr, span: u16) -> SocketAddr {
    if preferred.port() != 0 && is_addr_available(preferred) {
        return preferred;
    }
    let start = if preferred.port() == 0 {
        19080
    } else {
        preferred.port()
    };
    let ip = preferred.ip();
    for offset in 0..span {
        let Some(port) = start.checked_add(offset) else {
            break;
        };
        let candidate = SocketAddr::new(ip, port);
        if is_addr_available(candidate) {
            return candidate;
        }
    }
    preferred
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn profile_args(name: &str) -> cli::ConfigProfileSetArgs {
        cli::ConfigProfileSetArgs {
            name: name.to_string(),
            target: None,
            tcp_target: None,
            user: None,
            port: None,
            identity: Vec::new(),
            ssh_config: None,
            known_hosts: None,
            accept_new: false,
            insecure_ignore_host_key: false,
            no_accept_new: false,
            no_insecure_ignore_host_key: false,
            jump: Vec::new(),
            listen: None,
            remote_transport: None,
            remote_tcp: None,
            remote_control: None,
            remote_quic: None,
            remote_tls: None,
            remote_ca: None,
            remote_name: None,
            remote_client_cert: None,
            remote_client_key: None,
            remote_token: None,
            egress_proxy: None,
            allow_plain_tcp: false,
            no_allow_plain_tcp: false,
            transport_pool_size: None,
            workload_hint: None,
            ssh_session_pool_size: None,
            quic_max_bidi_streams: None,
            quic_stream_receive_window: None,
            quic_receive_window: None,
            quic_keep_alive_interval_secs: None,
            quic_idle_timeout_secs: None,
        }
    }

    #[test]
    fn profile_set_records_auth_and_peer_defaults() {
        let mut config = AppConfig::default();
        let mut args = profile_args("office");
        args.target = Some("user@app.internal".to_string());
        args.identity = vec![PathBuf::from("id_ed25519")];
        args.known_hosts = Some(PathBuf::from("known_hosts"));
        args.jump = vec!["bastion".to_string()];
        args.accept_new = true;
        args.remote_transport = Some("tls-tcp".to_string());
        args.remote_tls = Some("192.0.2.2:19082".parse().unwrap());
        args.remote_ca = Some(PathBuf::from("ca.pem"));
        args.remote_token = Some("token".to_string());

        super::profile::apply_profile_set(&mut config, args).unwrap();

        let profile = config.profiles.get("office").unwrap();
        assert_eq!(profile.target.as_deref(), Some("user@app.internal"));
        assert_eq!(profile.identity, vec![PathBuf::from("id_ed25519")]);
        assert_eq!(
            profile.known_hosts.as_deref(),
            Some(Path::new("known_hosts"))
        );
        assert_eq!(profile.jump, vec!["bastion"]);
        assert_eq!(profile.accept_new, Some(true));
        assert_eq!(profile.remote_transport.as_deref(), Some("tls-tcp"));
        assert_eq!(profile.remote_tls, Some("192.0.2.2:19082".parse().unwrap()));
        assert_eq!(profile.remote_ca.as_deref(), Some(Path::new("ca.pem")));
        assert_eq!(profile.remote_token.as_deref(), Some("token"));
    }

    #[test]
    fn profile_set_records_transport_pool_size() {
        let mut config = AppConfig::default();
        let mut args = profile_args("office");
        args.transport_pool_size = Some(4);
        args.workload_hint = Some(cli::RouteWorkloadHint::Mixed);

        super::profile::apply_profile_set(&mut config, args).unwrap();

        let profile = config.profiles.get("office").unwrap();
        assert_eq!(profile.transport_pool_size, Some(4));
        assert_eq!(profile.workload_hint, Some(cli::RouteWorkloadHint::Mixed));
    }

    #[test]
    fn proxy_defaults_cap_ssh_session_pool_but_profile_can_override() {
        let mut config = AppConfig::default();
        config.defaults.ssh_session_pool_size = Some(8);

        let mut args = super::profile::default_proxy_args("office".to_string());
        config
            .apply_proxy_defaults(&mut args, Some("office"))
            .unwrap();

        assert_eq!(args.ssh_session_pool_size, Some(2));
        assert_eq!(args.ssh_session_pool_source.as_deref(), Some("defaults"));
        assert!(
            args.ssh_session_pool_reason
                .as_deref()
                .expect("pool reason")
                .contains("capped to pool=2")
        );

        config.profiles.insert(
            "office".to_string(),
            ProxyProfile {
                ssh_session_pool_size: Some(4),
                ..Default::default()
            },
        );
        let mut args = super::profile::default_proxy_args("office".to_string());
        config
            .apply_proxy_defaults(&mut args, Some("office"))
            .unwrap();

        assert_eq!(args.ssh_session_pool_size, Some(4));
        assert_eq!(args.ssh_session_pool_source.as_deref(), Some("profile"));
        assert!(
            args.ssh_session_pool_warning
                .as_deref()
                .expect("pool warning")
                .contains("above 2")
        );
    }

    #[test]
    fn profile_set_records_quic_transport_options() {
        let mut config = AppConfig::default();
        let mut args = profile_args("office");
        args.quic_max_bidi_streams = Some(512);
        args.quic_stream_receive_window = Some(4 * 1024 * 1024);
        args.quic_receive_window = Some(32 * 1024 * 1024);
        args.quic_keep_alive_interval_secs = Some(20);
        args.quic_idle_timeout_secs = Some(120);

        super::profile::apply_profile_set(&mut config, args).unwrap();

        let profile = config.profiles.get("office").unwrap();
        assert_eq!(profile.quic_max_bidi_streams, Some(512));
        assert_eq!(profile.quic_stream_receive_window, Some(4 * 1024 * 1024));
        assert_eq!(profile.quic_receive_window, Some(32 * 1024 * 1024));
        assert_eq!(profile.quic_keep_alive_interval_secs, Some(20));
        assert_eq!(profile.quic_idle_timeout_secs, Some(120));
    }

    #[test]
    fn config_default_records_schema_version() {
        let config = AppConfig::default();

        assert_eq!(config.schema_version, CONFIG_SCHEMA_VERSION);
        assert!(toml::to_string(&config).unwrap().contains("schema_version"));
    }

    #[test]
    fn future_config_schema_is_rejected() {
        let config = AppConfig {
            schema_version: CONFIG_SCHEMA_VERSION + 1,
            ..Default::default()
        };

        let err = config.validate_schema().unwrap_err().to_string();

        assert!(err.contains("newer than this binary supports"));
    }

    #[test]
    fn config_inspect_redacts_secret_material() {
        let mut config = AppConfig::default();
        config.identity.node_id = Some("spx-local".to_string());
        config.identity.secret = Some("identity-secret".to_string());
        config.daemon.token = Some("daemon-secret".to_string());
        config.daemon.token_metadata = Some(TokenMetadata::new("daemon-control-transport"));
        config.profiles.insert(
            "office".to_string(),
            ProxyProfile {
                target: Some("office.example".to_string()),
                remote_token: Some("profile-secret".to_string()),
                remote_tcp: Some("127.0.0.1:19080".parse().unwrap()),
                ..Default::default()
            },
        );
        config.peers.insert(
            "office".to_string(),
            PeerRecord {
                node_id: Some("spx-office".to_string()),
                token: Some("peer-secret".to_string()),
                token_metadata: Some(TokenMetadata::new("peer-control-transport")),
                transport: Some("127.0.0.1:19080".parse().unwrap()),
                ..Default::default()
            },
        );

        let inspect = diagnostics::inspect(&config);
        let text = inspect.to_string();

        assert_eq!(inspect["kind"], "config_inspect");
        assert_eq!(inspect["identity"]["secret"], true);
        assert_eq!(inspect["daemon"]["auth"]["token"], true);
        assert_eq!(inspect["profiles"][0]["remote"]["token"], true);
        assert_eq!(inspect["peers"][0]["auth"]["token"], true);
        assert!(!text.contains("identity-secret"));
        assert!(!text.contains("daemon-secret"));
        assert!(!text.contains("profile-secret"));
        assert!(!text.contains("peer-secret"));
    }

    #[test]
    fn config_export_descriptor_is_redacted() {
        let mut config = AppConfig::default();
        config.identity.node_id = Some("spx-local".to_string());
        config.identity.node_name = Some("local".to_string());
        config.identity.secret = Some("identity-secret".to_string());
        config.daemon.control_endpoint = Some("tcp://127.0.0.1:19081".to_string());
        config.daemon.transport_listen = Some("127.0.0.1:19080".parse().unwrap());
        config.daemon.token = Some("daemon-secret".to_string());
        config.daemon.token_metadata = Some(TokenMetadata::new("daemon-control-transport"));

        let descriptor = diagnostics::export_descriptor(&config);
        let text = descriptor.to_string();

        assert_eq!(descriptor["kind"], "peer_descriptor");
        assert_eq!(descriptor["source"], "config-export");
        assert_eq!(descriptor["auth"]["control_token"], true);
        assert_eq!(descriptor["endpoints"]["control"], "tcp://127.0.0.1:19081");
        assert_eq!(descriptor["transport_protocols"][0], "plain-tcp");
        assert!(!text.contains("identity-secret"));
        assert!(!text.contains("daemon-secret"));
    }

    #[test]
    fn import_descriptor_records_peer_and_profile_without_embedded_secret() {
        let path = std::env::temp_dir().join(format!(
            "ssh_proxy-descriptor-import-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            serde_json::json!({
                "ok": true,
                "kind": "peer_descriptor",
                "node_id": "spx-remote",
                "node_name": "remote",
                "version": "0.2.0",
                "os": "linux",
                "arch": "x86_64",
                "control_api_version": 1,
                "peer_protocol_version": 1,
                "features": ["frames-v1", "token-auth-v1"],
                "endpoints": {
                    "control": "tcp://127.0.0.1:29081",
                    "transport": "127.0.0.1:29080",
                    "tls_transport": "127.0.0.1:29082"
                },
                "transport_protocols": ["tls-tcp", "plain-tcp"],
                "auth": {
                    "control_token": true,
                    "token_metadata": {
                        "created_at_unix": 42,
                        "rotated_at_unix": null,
                        "scope": "peer-control-transport",
                        "expires_at_unix": null
                    }
                }
            })
            .to_string(),
        )
        .unwrap();
        let mut config = AppConfig::default();
        let args = cli::ConfigImportDescriptorArgs {
            alias: "office".to_string(),
            path: path.display().to_string(),
            target: Some("office.example".to_string()),
            token: Some("out-of-band-token".to_string()),
            trust: "descriptor-import-test".to_string(),
        };

        diagnostics::import_peer_descriptor(&mut config, &args).unwrap();

        let peer = config.peers.get("office").unwrap();
        assert_eq!(peer.node_id.as_deref(), Some("spx-remote"));
        assert_eq!(peer.version.as_deref(), Some("0.2.0"));
        assert_eq!(peer.control_api_version, Some(1));
        assert_eq!(peer.peer_protocol_version, Some(1));
        assert_eq!(peer.features, vec!["frames-v1", "token-auth-v1"]);
        assert_eq!(peer.os.as_deref(), Some("linux"));
        assert_eq!(peer.arch.as_deref(), Some("x86_64"));
        assert_eq!(peer.target.as_deref(), Some("office.example"));
        assert_eq!(
            peer.control_endpoint.as_deref(),
            Some("tcp://127.0.0.1:29081")
        );
        assert_eq!(peer.transport, Some("127.0.0.1:29080".parse().unwrap()));
        assert_eq!(peer.tls_transport, Some("127.0.0.1:29082".parse().unwrap()));
        assert_eq!(peer.token.as_deref(), Some("out-of-band-token"));
        assert_eq!(peer.trust.as_deref(), Some("descriptor-import-test"));
        let profile = config.profiles.get("office").unwrap();
        assert_eq!(profile.remote_token.as_deref(), Some("out-of-band-token"));
        assert_eq!(
            profile.remote_control,
            Some("127.0.0.1:29081".parse().unwrap())
        );
        assert_eq!(profile.remote_tcp, Some("127.0.0.1:29080".parse().unwrap()));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cert_store_sanitizes_names_and_rejects_overwrite() {
        let name = super::certs::sanitize_store_name("../prod node").unwrap();
        assert_eq!(name, "_prod_node");

        let base = std::env::temp_dir().join(format!("ssh_proxy-cert-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let src = base.join("src.pem");
        std::fs::write(&src, "cert").unwrap();
        let copied = super::certs::copy_cert_arg(&base, "remote-ca.pem", Some(src.clone()), false)
            .unwrap()
            .unwrap();
        assert_eq!(copied, base.join("remote-ca.pem"));
        let err = super::certs::copy_cert_arg(&base, "remote-ca.pem", Some(src), false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("already exists"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn private_text_save_replaces_existing_file() {
        let base = std::env::temp_dir().join(format!("ssh_proxy-save-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let path = base.join("config.toml");

        save_text_file_private(&path, "first").unwrap();
        save_text_file_private(&path, "second").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        assert!(std::fs::read_dir(&base).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp")
        }));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn node_identity_is_generated_once() {
        let mut config = AppConfig::default();

        let first = config.ensure_node_identity().unwrap();
        let second = config.ensure_node_identity().unwrap();

        assert!(first.node_id.as_deref().unwrap().starts_with("spx-"));
        assert_eq!(first.node_id, second.node_id);
        assert_eq!(first.secret, second.secret);
        assert!(first.node_name.is_some());
    }

    #[test]
    fn daemon_token_metadata_is_created_and_rotated() {
        let mut config = AppConfig::default();

        let first = config.ensure_daemon_token().unwrap();
        let first_meta = config.daemon.token_metadata.clone().unwrap();
        let second = config.ensure_daemon_token().unwrap();

        assert_eq!(first, second);
        assert_eq!(first_meta.scope, "daemon-control-transport");
        assert!(first_meta.rotated_at_unix.is_none());

        let rotated = config.rotate_daemon_token().unwrap();
        let rotated_meta = config.daemon.token_metadata.clone().unwrap();

        assert_ne!(first, rotated);
        assert_eq!(rotated_meta.scope, "daemon-control-transport");
        assert!(rotated_meta.rotated_at_unix.is_some());
    }

    #[test]
    fn peer_record_updates_last_seen() {
        let mut config = AppConfig::default();

        config.record_peer(
            "office",
            PeerRecord {
                node_id: Some("spx-peer".to_string()),
                trust: Some("ssh-bootstrap".to_string()),
                transport: Some("127.0.0.1:19080".parse().unwrap()),
                ..Default::default()
            },
        );

        let peer = config.peers.get("office").unwrap();
        assert_eq!(peer.node_id.as_deref(), Some("spx-peer"));
        assert_eq!(peer.trust.as_deref(), Some("ssh-bootstrap"));
        assert_eq!(peer.transport_protocols, vec!["plain-tcp"]);
        assert!(peer.last_seen_unix.is_some());
    }

    #[test]
    fn peer_record_orders_known_transport_protocols() {
        let peer = PeerRecord {
            transport: Some("127.0.0.1:19080".parse().unwrap()),
            tls_transport: Some("127.0.0.1:19082".parse().unwrap()),
            quic_transport: Some("127.0.0.1:19083".parse().unwrap()),
            ..Default::default()
        };

        assert_eq!(
            peer.known_transport_protocols(),
            vec!["quic", "tls-tcp", "plain-tcp"]
        );
    }
}
