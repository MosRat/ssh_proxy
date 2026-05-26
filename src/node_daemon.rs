use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Instant,
};

use anyhow::{Result, bail};
use serde_json::{Value, json};
use tokio::{
    sync::{Mutex, Notify},
    task::JoinHandle,
    time::{self, Duration},
};
use tracing::{error, info, warn};

use crate::{cli, config, control_socket, controller, paths, peer_transport, sidecar};

mod args;
mod control_client;
mod control_protocol;
mod control_server;
mod peers;
mod quic_transport;
mod routes;
mod transport;

use routes::RouteTask;

pub(crate) use args::{proxy_args_from_node_forward, reverse_args_from_node_reverse};
pub(crate) use control_protocol::{NodeRequest, NodeResponse, attach_auth_token, response_line};

pub(crate) fn control_api_version() -> u16 {
    control_protocol::NODE_CONTROL_VERSION
}

pub(crate) fn peer_protocol_version() -> u16 {
    peer_transport::PEER_VERSION
}

pub(crate) fn peer_protocol_features() -> Vec<String> {
    peer_transport::default_features()
}

fn feature_bits() -> Value {
    Value::Object(peer_transport::default_feature_bits())
}

fn service_instance_id(node_id: Option<&str>, os_user: &str, control_endpoint: &str) -> String {
    format!(
        "{}@{}:{}",
        node_id.unwrap_or("uninitialized-node"),
        os_user,
        control_endpoint
    )
}

pub async fn run(args: cli::NodeArgs, config: config::AppConfig) -> Result<()> {
    match args.command {
        cli::NodeCommand::Daemon(args) => run_daemon(args, config).await,
        cli::NodeCommand::Control(args) => control_client::run(args, config).await,
    }
}

async fn run_daemon(args: cli::NodeDaemonArgs, config: config::AppConfig) -> Result<()> {
    let endpoint = control_socket::ControlEndpoint::parse(&args.control)?;
    let manager = Arc::new(NodeManager::new(args, endpoint.clone(), config)?);

    let mut tasks = Vec::new();
    tasks.push(tokio::spawn(control_server::run_control_server(
        endpoint,
        manager.clone(),
    )));

    if let Some(transport) = manager.transport {
        tasks.push(tokio::spawn(transport::run_transport_listener(
            transport,
            manager.clone(),
        )));
    }

    if let Some(transport) = manager.tls_transport {
        tasks.push(tokio::spawn(transport::run_tls_transport_listener(
            transport,
            manager.clone(),
        )));
    }
    if let Some(transport) = manager.quic_transport {
        tasks.push(tokio::spawn(transport::run_quic_transport_listener(
            transport,
            manager.clone(),
        )));
    }

    if !manager.report_to.is_empty() {
        tasks.push(tokio::spawn(run_reporter(manager.clone())));
    }

    if manager.route_autostart {
        manager.restore_routes().await;
    }

    info!(
        name = %manager.name,
        control = %manager.control_endpoint,
        transport = ?manager.transport,
        tls_transport = ?manager.tls_transport,
        "node daemon started"
    );

    manager.shutdown_notified().await;
    for task in tasks {
        task.abort();
    }
    Ok(())
}

struct NodeManager {
    name: String,
    control_endpoint: control_socket::ControlEndpoint,
    transport: Option<SocketAddr>,
    tls_transport: Option<SocketAddr>,
    quic_transport: Option<SocketAddr>,
    quic_options: peer_transport::QuicTransportOptions,
    tls_cert: Option<PathBuf>,
    tls_key: Option<PathBuf>,
    tls_client_ca: Option<PathBuf>,
    token: RwLock<Option<String>>,
    report_to: Vec<String>,
    report_interval: Duration,
    config: Mutex<config::AppConfig>,
    started: Instant,
    shutdown: AtomicBool,
    shutdown_notify: Notify,
    active_transports: AtomicU32,
    total_transports: AtomicU32,
    instances: Mutex<HashMap<String, JoinHandle<()>>>,
    routes: Mutex<HashMap<String, RouteTask>>,
    route_store_path: PathBuf,
    route_autostart: bool,
    peer_reports: Mutex<HashMap<String, Value>>,
}

impl NodeManager {
    fn new(
        args: cli::NodeDaemonArgs,
        control_endpoint: control_socket::ControlEndpoint,
        mut config: config::AppConfig,
    ) -> Result<Self> {
        if config.identity.node_id.is_none()
            || config.identity.node_name.is_none()
            || config.identity.secret.is_none()
        {
            config.ensure_node_identity()?;
        }
        let quic_options = quic_options_from_daemon_args(&args, &config)?;
        let route_store_path = args
            .routes_path
            .or_else(|| config.daemon.routes_path.as_ref().map(config::expand_path))
            .unwrap_or(config::routes_path()?);
        let route_autostart =
            !args.no_route_autostart && config.daemon.route_autostart.unwrap_or(true);
        let transport = args.transport.or(config.daemon.transport_listen);
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
        let token = args.token.or_else(|| config.daemon.token.clone());
        let report_to = if args.report_to.is_empty() {
            config.daemon.report_to.clone()
        } else {
            args.report_to
        };
        if config.daemon.control_endpoint.is_none() && config.daemon.control_listen.is_none() {
            config.daemon.control_endpoint = Some(control_endpoint.to_string());
        }
        if config.daemon.transport_listen.is_none() && transport.is_some() {
            config.daemon.transport_listen = transport;
        }
        if config.daemon.tls_transport_listen.is_none() && tls_transport.is_some() {
            config.daemon.tls_transport_listen = tls_transport;
        }
        if config.daemon.quic_transport_listen.is_none() && quic_transport.is_some() {
            config.daemon.quic_transport_listen = quic_transport;
        }
        if config.daemon.token.is_none() && token.is_some() {
            config.daemon.token = token.clone();
        }
        if config.daemon.token.is_some() && config.daemon.token_metadata.is_none() {
            config.daemon.token_metadata =
                Some(config::TokenMetadata::new("daemon-control-transport"));
        }
        Ok(Self {
            name: args.name.unwrap_or_else(default_node_name),
            control_endpoint,
            transport,
            tls_transport,
            quic_transport,
            quic_options,
            tls_cert,
            tls_key,
            tls_client_ca,
            token: RwLock::new(token),
            report_to,
            report_interval: Duration::from_secs(args.report_interval_secs.max(1)),
            config: Mutex::new(config),
            started: Instant::now(),
            shutdown: AtomicBool::new(false),
            shutdown_notify: Notify::new(),
            active_transports: AtomicU32::new(0),
            total_transports: AtomicU32::new(0),
            instances: Mutex::new(HashMap::new()),
            routes: Mutex::new(HashMap::new()),
            route_store_path,
            route_autostart,
            peer_reports: Mutex::new(HashMap::new()),
        })
    }

    async fn connect_profile(&self, profile: &str) -> Result<String> {
        let mut instances = self.instances.lock().await;
        instances.retain(|_, handle| !handle.is_finished());
        if instances.contains_key(profile) {
            return Ok(format!("profile {profile:?} is already running"));
        }
        let mut args = {
            let config = self.config.lock().await;
            config.proxy_from_profile(profile)?
        };
        args.control_listen = None;
        let name = profile.to_string();
        let task_name = name.clone();
        let handle = tokio::spawn(async move {
            if let Err(err) = controller::run(args).await {
                error!(profile = %task_name, error = %err, "node profile instance stopped");
            }
        });
        instances.insert(name, handle);
        Ok(format!("profile {profile:?} started"))
    }

    async fn disconnect_profile(&self, profile: &str) -> Result<String> {
        let mut instances = self.instances.lock().await;
        if let Some(handle) = instances.remove(profile) {
            handle.abort();
            Ok(format!("profile {profile:?} stopped"))
        } else {
            bail!("profile {profile:?} is not running");
        }
    }

    async fn status_value(&self) -> Result<Value> {
        let mut instances = self.instances.lock().await;
        instances.retain(|_, handle| !handle.is_finished());
        let running = instances.keys().cloned().collect::<Vec<_>>();
        let routes = self.routes.lock().await;
        let mut running_routes = Vec::with_capacity(routes.len());
        for (id, task) in routes.iter() {
            let stats = task.stats.lock().await.clone();
            let link = match &task.link_state {
                Some(state) => state.status_value().await,
                None => Value::Null,
            };
            running_routes.push(json!({
                "id": id,
                "direction": task.direction,
                "detail": task.detail,
                "listen": task.listen.map(|addr| addr.to_string()),
                "peer": task.peer,
                "persist": task.persist,
                "created_at_unix": task.created_at_unix,
                "fallback_reason": task.fallback_reason.clone(),
                "task_finished": task.handle.is_finished(),
                "runtime": task.spec.runtime_metadata(),
                "stats": stats,
                "link": link,
            }));
        }
        drop(routes);
        let reports = self.peer_reports.lock().await.clone();
        let (node_id, node_name, profiles, peers, token_metadata) = {
            let config = self.config.lock().await;
            (
                config.identity.node_id.clone(),
                config
                    .identity
                    .node_name
                    .clone()
                    .unwrap_or_else(|| self.name.clone()),
                config.profiles.keys().cloned().collect::<Vec<_>>(),
                config.peers.keys().cloned().collect::<Vec<_>>(),
                config.daemon.token_metadata.clone(),
            )
        };
        let os_user = whoami::username().unwrap_or_else(|_| "unknown-user".to_string());
        let data_dir = paths::app_home().ok();
        let service_instance_id = service_instance_id(
            node_id.as_deref(),
            &os_user,
            &self.control_endpoint.to_string(),
        );
        let token_generation = token_metadata.as_ref().map(|metadata| metadata.generation);
        let tls_server_cert_fingerprint = self
            .tls_cert
            .as_ref()
            .and_then(|path| config::file_sha256_fingerprint(path));
        let tls_client_ca_fingerprint = self
            .tls_client_ca
            .as_ref()
            .and_then(|path| config::file_sha256_fingerprint(path));
        let has_token = self.token_value().is_some();
        Ok(json!({
            "api_version": control_protocol::NODE_CONTROL_VERSION,
            "ok": true,
            "kind": "node",
            "name": self.name,
            "version": env!("CARGO_PKG_VERSION"),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "linux_musl_sidecar": sidecar::build_summary(),
            "node_id": node_id,
            "node_name": node_name,
            "service_instance_id": service_instance_id,
            "uptime_secs": self.started.elapsed().as_secs(),
            "os_user": os_user,
            "data_dir": data_dir,
            "control": self.control_endpoint.to_string(),
            "transport": self.transport.map(|addr| addr.to_string()),
            "tls_transport": self.tls_transport.map(|addr| addr.to_string()),
            "quic_transport": self.quic_transport.map(|addr| addr.to_string()),
            "quic_transport_options": self.quic_options,
            "quic_max_bidi_streams": self.quic_options.max_bidi_streams,
            "quic_stream_receive_window": self.quic_options.stream_receive_window,
            "quic_receive_window": self.quic_options.receive_window,
            "quic_keep_alive_interval_secs": self.quic_options.keep_alive_interval_secs,
            "quic_idle_timeout_secs": self.quic_options.idle_timeout_secs,
            "quic_runtime": peer_transport::quic_runtime_diagnostics(self.quic_options),
            "quic_udp_runtime": peer_transport::QUIC_UDP_RUNTIME,
            "quic_udp_gso": Value::Null,
            "quic_udp_gso_source": peer_transport::QUIC_UDP_GSO_SOURCE,
            "quic_packetization": peer_transport::QUIC_PACKETIZATION,
            "auth": {
                "control_token": has_token,
                "transport_token": has_token,
                "token_metadata": token_metadata,
                "token_generation": token_generation,
                "tls_server_cert": self.tls_cert.is_some() && self.tls_key.is_some(),
                "tls_client_ca": self.tls_client_ca.is_some(),
                "tls_server_cert_fingerprint": tls_server_cert_fingerprint,
                "tls_client_ca_fingerprint": tls_client_ca_fingerprint,
            },
            "active_transports": self.active_transports.load(Ordering::Relaxed),
            "total_transports": self.total_transports.load(Ordering::Relaxed),
            "profiles": profiles,
            "peers": peers,
            "running": running,
            "routes": running_routes,
            "route_store": self.route_store_path,
            "route_autostart": self.route_autostart,
            "report_to": self.report_to,
            "peer_reports": reports,
        }))
    }

    async fn status_json(&self) -> Result<String> {
        response_line(self.status_value().await?)
    }

    async fn descriptor_value(&self) -> Result<Value> {
        let (node_id, node_name, token_metadata) = {
            let config = self.config.lock().await;
            (
                config.identity.node_id.clone(),
                config
                    .identity
                    .node_name
                    .clone()
                    .unwrap_or_else(|| self.name.clone()),
                config.daemon.token_metadata.clone(),
            )
        };
        let os_user = whoami::username().unwrap_or_else(|_| "unknown-user".to_string());
        let data_dir = paths::app_home().ok();
        let service_instance_id = service_instance_id(
            node_id.as_deref(),
            &os_user,
            &self.control_endpoint.to_string(),
        );
        let token_generation = token_metadata.as_ref().map(|metadata| metadata.generation);
        let tls_server_cert_fingerprint = self
            .tls_cert
            .as_ref()
            .and_then(|path| config::file_sha256_fingerprint(path));
        let tls_client_ca_fingerprint = self
            .tls_client_ca
            .as_ref()
            .and_then(|path| config::file_sha256_fingerprint(path));
        let has_token = self.token_value().is_some();
        Ok(json!({
            "ok": true,
            "kind": "peer_descriptor",
            "name": self.name,
            "node_id": node_id,
            "node_name": node_name,
            "service_instance_id": service_instance_id,
            "version": env!("CARGO_PKG_VERSION"),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "os_user": os_user,
            "data_dir": data_dir,
            "control_api_version": control_protocol::NODE_CONTROL_VERSION,
            "peer_protocol_version": peer_transport::PEER_VERSION,
            "features": peer_transport::default_features(),
            "feature_bits": feature_bits(),
            "control_endpoint": self.control_endpoint.to_string(),
            "endpoints": {
                "control": self.control_endpoint.to_string(),
                "transport": self.transport.map(|addr| addr.to_string()),
                "tls_transport": self.tls_transport.map(|addr| addr.to_string()),
                "quic_transport": self.quic_transport.map(|addr| addr.to_string()),
            },
            "transport_protocols": self.transport_protocols(),
            "quic_transport_options": self.quic_options,
            "quic_runtime": peer_transport::quic_runtime_diagnostics(self.quic_options),
            "auth": {
                "control_token": has_token,
                "transport_token": has_token,
                "token_metadata": token_metadata,
                "token_generation": token_generation,
                "tls_server_cert": self.tls_cert.is_some() && self.tls_key.is_some(),
                "tls_client_ca": self.tls_client_ca.is_some(),
                "tls_server_cert_fingerprint": tls_server_cert_fingerprint,
                "tls_client_ca_fingerprint": tls_client_ca_fingerprint,
            },
            "routes_path": self.route_store_path,
            "route_autostart": self.route_autostart,
            "linux_musl_sidecar": sidecar::build_summary(),
        }))
    }

    async fn descriptor_json(&self) -> Result<String> {
        response_line(self.descriptor_value().await?)
    }

    async fn links_json(&self) -> Result<String> {
        let status = self.status_value().await?;
        let links = json!({
            "ok": true,
            "name": self.name,
            "transport": status["transport"].clone(),
            "active_transports": status["active_transports"].clone(),
            "total_transports": status["total_transports"].clone(),
            "running": status["running"].clone(),
            "routes": status["routes"].clone(),
            "peer_reports": status["peer_reports"].clone(),
        });
        response_line(links)
    }

    fn token_value(&self) -> Option<String> {
        self.token.read().ok().and_then(|token| token.clone())
    }

    async fn rotate_token(&self) -> Result<String> {
        let (token, metadata) = {
            let mut config = self.config.lock().await;
            let token = config.rotate_daemon_token()?;
            let metadata = config.daemon.token_metadata.clone();
            config.save_default()?;
            (token, metadata)
        };
        if let Ok(mut current) = self.token.write() {
            *current = Some(token.clone());
        }
        response_line(json!({
            "ok": true,
            "message": "daemon token rotated",
            "token": token,
            "token_metadata": metadata,
            "next_action": "update clients or rerun node control after local config reload",
        }))
    }

    fn transport_protocols(&self) -> Vec<String> {
        let mut protocols = Vec::new();
        if self.quic_transport.is_some() {
            protocols.push("quic".to_string());
        }
        if self.tls_transport.is_some() {
            protocols.push("tls-tcp".to_string());
        }
        if self.transport.is_some() {
            protocols.push("plain-tcp".to_string());
        }
        protocols
    }

    async fn shutdown(&self) -> Result<String> {
        self.shutdown.store(true, Ordering::Relaxed);
        let mut instances = self.instances.lock().await;
        for (_, handle) in instances.drain() {
            handle.abort();
        }
        let mut routes = self.routes.lock().await;
        for (_, task) in routes.drain() {
            task.handle.abort();
        }
        self.shutdown_notify.notify_waiters();
        NodeResponse::ok_message("node shutdown requested").to_line()
    }

    async fn shutdown_notified(&self) {
        if self.shutdown.load(Ordering::Relaxed) {
            return;
        }
        self.shutdown_notify.notified().await;
    }
}

async fn run_reporter(manager: Arc<NodeManager>) -> Result<()> {
    loop {
        tokio::select! {
            _ = time::sleep(manager.report_interval) => {}
            _ = manager.shutdown_notified() => break,
        }
        let status = match manager.status_value().await {
            Ok(status) => status,
            Err(err) => {
                warn!(error = %err, "failed to build node status report");
                continue;
            }
        };
        for endpoint in &manager.report_to {
            let Ok(endpoint) = control_socket::ControlEndpoint::parse(endpoint) else {
                warn!(endpoint, "invalid node report endpoint");
                continue;
            };
            let request = NodeRequest::report(manager.name.clone(), status.clone()).to_line()?;
            if let Err(err) = control_socket::request(&endpoint, &request).await {
                warn!(%endpoint, error = %err, "failed to report node status");
            }
        }
    }
    Ok(())
}

fn default_node_name() -> String {
    let user = whoami::username().unwrap_or_else(|_| "unknown".to_string());
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown".to_string());
    format!("{user}@{host}")
}

fn quic_options_from_daemon_args(
    args: &cli::NodeDaemonArgs,
    config: &config::AppConfig,
) -> Result<peer_transport::QuicTransportOptions> {
    let defaults = peer_transport::QuicTransportOptions::default();
    peer_transport::QuicTransportOptions::new(
        args.quic_max_bidi_streams
            .or(config.daemon.quic_max_bidi_streams)
            .unwrap_or(defaults.max_bidi_streams),
        args.quic_stream_receive_window
            .or(config.daemon.quic_stream_receive_window)
            .unwrap_or(defaults.stream_receive_window),
        args.quic_receive_window
            .or(config.daemon.quic_receive_window)
            .unwrap_or(defaults.receive_window),
        args.quic_keep_alive_interval_secs
            .or(config.daemon.quic_keep_alive_interval_secs)
            .unwrap_or(defaults.keep_alive_interval_secs),
        args.quic_idle_timeout_secs
            .or(config.daemon.quic_idle_timeout_secs)
            .unwrap_or(defaults.idle_timeout_secs),
    )
}
