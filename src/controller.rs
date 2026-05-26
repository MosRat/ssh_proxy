use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{Mutex, Notify, RwLock},
    task::JoinHandle,
    time,
};
use tracing::{debug, error, info, warn};

use crate::{
    bridge, cli, config, control_socket, deploy, peer_transport, protocol, quic_native, sidecar,
    socks, ssh_native,
};

pub async fn run(args: cli::ProxyArgs) -> Result<()> {
    match args.remote_transport {
        cli::RemoteTransport::SshNative => {
            let state = ssh_native::State::new(args.clone());
            run_ssh_native_with_state(args, state).await
        }
        cli::RemoteTransport::QuicNative => {
            let state = quic_native::State::connect(args.clone()).await?;
            run_quic_native_with_state(args, state).await
        }
        _ => {
            let state = shared_state(&args);
            run_with_state(args, state).await
        }
    }
}

pub async fn run_ssh_native_with_state(
    args: cli::ProxyArgs,
    state: Arc<ssh_native::State>,
) -> Result<()> {
    ssh_native::run_with_state(args, state).await
}

pub async fn run_quic_native_with_state(
    args: cli::ProxyArgs,
    state: Arc<quic_native::State>,
) -> Result<()> {
    quic_native::run_with_state(args, state).await
}

pub async fn run_quic_native_with_slot(
    args: cli::ProxyArgs,
    slot: Arc<quic_native::StateSlot>,
) -> Result<()> {
    quic_native::run_with_slot(args, slot).await
}

pub fn shared_state(args: &cli::ProxyArgs) -> Arc<SharedState> {
    let quic_options = peer_transport::QuicTransportOptions::new(
        args.quic_max_bidi_streams,
        args.quic_stream_receive_window,
        args.quic_receive_window,
        args.quic_keep_alive_interval_secs,
        args.quic_idle_timeout_secs,
    )
    .unwrap_or_default();
    Arc::new(SharedState::new(
        !args.no_reconnect,
        args.egress_proxy.clone(),
        args.transport_pool_size,
        quic_options,
        tls_peer_auth_mode(args),
        args.pool_policy.clone(),
        args.workload_hint
            .map(workload_hint_name)
            .map(str::to_string),
    ))
}

fn tls_peer_auth_mode(args: &cli::ProxyArgs) -> Option<String> {
    if !matches!(args.remote_transport, cli::RemoteTransport::TlsTcp) {
        return None;
    }
    Some(
        match (
            args.remote_client_cert.is_some(),
            args.remote_client_key.is_some(),
        ) {
            (true, true) => "mutual_tls",
            (false, false) => "server_auth",
            _ => "invalid_client_auth_config",
        }
        .to_string(),
    )
}

fn workload_hint_name(hint: cli::RouteWorkloadHint) -> &'static str {
    match hint {
        cli::RouteWorkloadHint::Large => "large",
        cli::RouteWorkloadHint::Concurrent => "concurrent",
        cli::RouteWorkloadHint::Mixed => "mixed",
    }
}

pub async fn run_with_state(args: cli::ProxyArgs, state: Arc<SharedState>) -> Result<()> {
    if matches!(
        args.remote_transport,
        cli::RemoteTransport::Auto | cli::RemoteTransport::Exec
    ) {
        warn!(
            "proxy may fall back to a temporary SSH exec helper; for production prefer `ssh_proxy service ... install` locally plus `ssh_proxy host ... --persist auto start` remotely"
        );
    }
    let tcp_target = args.tcp_target.clone();
    let manager_state = state.clone();
    let manager_args = args.clone();
    tokio::spawn(async move {
        run_bridge_manager(manager_args, manager_state).await;
    });
    if args.no_reconnect {
        state
            .wait_for_initial_bridge(Duration::from_secs(args.connect_timeout_secs.max(1)))
            .await?;
    }

    if let Some(addr) = args.control_listen {
        let control_state = state.clone();
        tokio::spawn(async move {
            if let Err(err) = run_control_server(addr, control_state).await {
                error!(%addr, error = %err, "control server stopped");
            }
        });
    }

    let listener = TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("failed to bind SOCKS listener {}", args.listen))?;
    info!(listen = %args.listen, "SOCKS5H proxy listening");

    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, peer) = accept?;
                let state = state.clone();
                let tcp_target = tcp_target.clone();
                tokio::spawn(async move {
                    let result = if let Some(target) = tcp_target {
                        socks::handle_fixed_target(stream, peer, target, state).await
                    } else {
                        socks::handle_client(stream, peer, state).await
                    };
                    if let Err(err) = result {
                        debug!(%peer, error = %err, "SOCKS client failed");
                    }
                });
            }
            _ = state.shutdown_notified() => {
                info!("shutdown requested; stopping SOCKS listener");
                break;
            }
        }
    }
    Ok(())
}

pub async fn control(mut args: cli::ControlArgs, config: config::AppConfig) -> Result<()> {
    let default_addr = SocketAddr::from(([127, 0, 0, 1], 1081));
    if args.endpoint.is_none() && args.addr == default_addr {
        if let Some(addr) = config.daemon.control_listen {
            args.addr = addr;
        }
    }
    let endpoint = match args
        .endpoint
        .as_deref()
        .or(config.daemon.control_endpoint.as_deref())
    {
        Some(value) => control_socket::ControlEndpoint::parse(value)?,
        None => control_socket::ControlEndpoint::from_addr(args.addr),
    };
    let command = match args.command {
        cli::ControlCommand::Status => "status\n".to_string(),
        cli::ControlCommand::Shutdown => "shutdown\n".to_string(),
        cli::ControlCommand::Connect { profile } => {
            format!(
                "{}\n",
                serde_json::json!({"cmd": "connect", "profile": profile})
            )
        }
        cli::ControlCommand::Disconnect { profile } => {
            format!(
                "{}\n",
                serde_json::json!({"cmd": "disconnect", "profile": profile})
            )
        }
    };
    let response = control_socket::request(&endpoint, &command).await?;
    print!("{response}");
    Ok(())
}

pub async fn daemon(mut args: cli::DaemonArgs, config: config::AppConfig) -> Result<()> {
    let default_addr = SocketAddr::from(([127, 0, 0, 1], 1081));
    if args.control.is_none() && args.control_listen == default_addr {
        if let Some(addr) = config.daemon.control_listen {
            args.control_listen = addr;
        }
    }
    let endpoint = match args
        .control
        .as_deref()
        .or(config.daemon.control_endpoint.as_deref())
    {
        Some(value) => control_socket::ControlEndpoint::parse(value)?,
        None => control_socket::ControlEndpoint::from_addr(args.control_listen),
    };
    let manager = Arc::new(DaemonManager::new(config));
    run_daemon_control_server(endpoint, manager).await
}

struct DaemonManager {
    config: config::AppConfig,
    started: Instant,
    instances: Mutex<HashMap<String, JoinHandle<()>>>,
    shutdown: AtomicBool,
    shutdown_notify: Notify,
}

impl DaemonManager {
    fn new(config: config::AppConfig) -> Self {
        Self {
            config,
            started: Instant::now(),
            instances: Mutex::new(HashMap::new()),
            shutdown: AtomicBool::new(false),
            shutdown_notify: Notify::new(),
        }
    }

    async fn connect_profile(&self, profile: &str) -> Result<String> {
        let mut instances = self.instances.lock().await;
        instances.retain(|_, handle| !handle.is_finished());
        if instances.contains_key(profile) {
            return Ok(format!("profile {profile:?} is already running"));
        }
        let mut args = self.config.proxy_from_profile(profile)?;
        args.control_listen = None;
        let name = profile.to_string();
        let task_name = name.clone();
        let handle = tokio::spawn(async move {
            if let Err(err) = run(args).await {
                error!(profile = %task_name, error = %err, "daemon proxy instance stopped");
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

    async fn status_json(&self) -> Result<String> {
        #[derive(Serialize)]
        struct DaemonStatus {
            ok: bool,
            version: &'static str,
            os: &'static str,
            arch: &'static str,
            linux_musl_sidecar: &'static str,
            uptime_secs: u64,
            config_path: String,
            profiles: Vec<String>,
            running: Vec<String>,
        }

        let profiles = self.config.profiles.keys().cloned().collect::<Vec<_>>();
        let mut instances = self.instances.lock().await;
        instances.retain(|_, handle| !handle.is_finished());
        let running = instances.keys().cloned().collect::<Vec<_>>();
        let status = DaemonStatus {
            ok: true,
            version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            linux_musl_sidecar: sidecar::build_summary(),
            uptime_secs: self.started.elapsed().as_secs(),
            config_path: config::config_path()?.display().to_string(),
            profiles,
            running,
        };
        Ok(format!("{}\n", serde_json::to_string_pretty(&status)?))
    }

    async fn shutdown(&self) -> Result<String> {
        self.shutdown.store(true, Ordering::Relaxed);
        let mut instances = self.instances.lock().await;
        for (_, handle) in instances.drain() {
            handle.abort();
        }
        self.shutdown_notify.notify_waiters();
        Ok("{\"ok\":true,\"message\":\"daemon shutdown requested\"}\n".to_string())
    }

    async fn shutdown_notified(&self) {
        if self.shutdown.load(Ordering::Relaxed) {
            return;
        }
        self.shutdown_notify.notified().await;
    }
}

#[derive(Deserialize)]
struct DaemonRequest {
    cmd: String,
    profile: Option<String>,
}

async fn run_daemon_control_server(
    endpoint: control_socket::ControlEndpoint,
    manager: Arc<DaemonManager>,
) -> Result<()> {
    let listener = control_socket::ControlListener::bind(&endpoint).await?;
    info!(%endpoint, "daemon control listener ready");
    loop {
        tokio::select! {
            accept = listener.accept() => {
                let stream = accept?;
                let manager = manager.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_daemon_control(stream, manager).await {
                        warn!(error = %err, "daemon control request failed");
                    }
                });
            }
            _ = manager.shutdown_notified() => break,
        }
    }
    Ok(())
}

async fn handle_daemon_control(
    stream: control_socket::ControlStream,
    manager: Arc<DaemonManager>,
) -> Result<()> {
    let mut reader = BufReader::new(stream);
    let mut command = String::new();
    reader.read_line(&mut command).await?;
    let mut stream = reader.into_inner();
    let response = match parse_daemon_request(&command) {
        Ok(request) => match request.cmd.to_ascii_lowercase().as_str() {
            "status" | "" => manager.status_json().await?,
            "shutdown" => manager.shutdown().await?,
            "connect" => {
                let profile = request
                    .profile
                    .ok_or_else(|| anyhow!("connect requires a profile"))?;
                let message = manager.connect_profile(&profile).await?;
                format!("{}\n", serde_json::json!({"ok": true, "message": message}))
            }
            "disconnect" => {
                let profile = request
                    .profile
                    .ok_or_else(|| anyhow!("disconnect requires a profile"))?;
                let message = manager.disconnect_profile(&profile).await?;
                format!("{}\n", serde_json::json!({"ok": true, "message": message}))
            }
            other => format!(
                "{}\n",
                serde_json::json!({
                    "ok": false,
                    "error": format!("unknown daemon command {other:?}")
                })
            ),
        },
        Err(err) => format!(
            "{}\n",
            serde_json::json!({"ok": false, "error": err.to_string()})
        ),
    };
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await.ok();
    Ok(())
}

fn parse_daemon_request(command: &str) -> Result<DaemonRequest> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Ok(DaemonRequest {
            cmd: "status".to_string(),
            profile: None,
        });
    }
    if trimmed.starts_with('{') {
        return serde_json::from_str(trimmed).context("failed to parse daemon JSON request");
    }
    Ok(DaemonRequest {
        cmd: trimmed.to_string(),
        profile: None,
    })
}

async fn run_bridge_manager(args: cli::ProxyArgs, state: Arc<SharedState>) {
    let pool_size = args.transport_pool_size.max(1);
    info!(pool_size, "starting route peer transport pool");
    let mut workers = Vec::with_capacity(pool_size);
    for slot in 0..pool_size {
        let args = args.clone();
        let state = state.clone();
        workers.push(tokio::spawn(async move {
            run_bridge_worker(slot, args, state).await;
        }));
    }
    for worker in workers {
        worker.await.ok();
    }
}

async fn run_bridge_worker(slot: usize, args: cli::ProxyArgs, state: Arc<SharedState>) {
    let mut delay = Duration::from_secs(args.reconnect_delay_secs);
    let max_delay =
        Duration::from_secs(args.reconnect_max_delay_secs.max(args.reconnect_delay_secs));
    loop {
        if state.shutdown.load(Ordering::Relaxed) {
            break;
        }
        let attempt = state.record_bridge_attempt(slot).await;
        info!(slot, attempt, "connecting remote bridge");
        let connect = time::timeout(
            Duration::from_secs(args.connect_timeout_secs.max(1)),
            bridge::Bridge::connect_via_ssh(&args),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "remote bridge connection timed out after {}s",
                args.connect_timeout_secs
            )
        })
        .and_then(|result| result);
        match connect {
            Ok(bridge) => {
                let generation = state.generation.fetch_add(1, Ordering::Relaxed) + 1;
                let selected_protocol = bridge.selected_protocol;
                let transport_timings = bridge.transport_timings;
                state.set_candidate_failures(Vec::new()).await;
                state
                    .record_bridge_connected(slot, generation, selected_protocol, transport_timings)
                    .await;
                delay = Duration::from_secs(args.reconnect_delay_secs);
                info!(slot, generation, "remote bridge connected");
                state.set_bridge(slot, Some(bridge.handle.clone())).await;
                tokio::select! {
                    _ = bridge.lifecycle => {}
                    _ = state.shutdown_notified() => {}
                }
                warn!(slot, generation, "remote bridge disconnected");
                state.record_bridge_disconnected(slot).await;
                state.set_bridge(slot, None).await;
            }
            Err(err) => {
                let candidate_failures = err
                    .downcast_ref::<deploy::AutoTransportError>()
                    .map(|err| err.failures.clone());
                let detail = format!("{err:#}");
                state.record_bridge_failed(slot, detail.clone()).await;
                if let Some(failures) = candidate_failures {
                    state.set_candidate_failures(failures).await;
                }
                error!(slot, attempt, error = %detail, "failed to connect remote bridge");
                state.set_bridge(slot, None).await;
            }
        }

        if !state.reconnect {
            info!("reconnect disabled; bridge manager exiting");
            break;
        }
        let sleep_delay = jittered_backoff(delay, max_delay);
        warn!(
            slot,
            base_retry_secs = delay.as_secs(),
            next_retry_secs = sleep_delay.as_secs_f64(),
            "retrying remote bridge connection after jittered backoff"
        );
        tokio::select! {
            _ = time::sleep(sleep_delay) => {}
            _ = state.shutdown_notified() => break,
        }
        delay = (delay * 2).min(max_delay);
    }
}

pub struct SharedState {
    bridge_slots: RwLock<Vec<Option<bridge::BridgeHandle>>>,
    bridge_workers: RwLock<Vec<BridgeWorkerState>>,
    bridge_worker_counters: Vec<Arc<BridgeWorkerCounters>>,
    notify: Notify,
    shutdown_notify: Notify,
    shutdown: AtomicBool,
    reconnect: bool,
    transport_pool_size: usize,
    quic_options: peer_transport::QuicTransportOptions,
    tls_peer_auth_mode: Option<String>,
    pool_policy: Option<String>,
    workload_hint: Option<String>,
    next_bridge: AtomicU32,
    started: Instant,
    generation: AtomicU32,
    connect_attempts: AtomicU32,
    successful_connects: AtomicU32,
    failed_connects: AtomicU32,
    active_tcp: AtomicU32,
    total_tcp: AtomicU32,
    tcp_open_attempts: AtomicU64,
    tcp_open_successes: AtomicU64,
    tcp_open_failures: AtomicU64,
    last_tcp_open_latency_ms: AtomicU64,
    ssh_direct_channel_open_samples: AtomicU64,
    last_ssh_direct_channel_open_latency_ms: AtomicU64,
    spx_peer_handshake_samples: AtomicU64,
    last_spx_peer_handshake_latency_ms: AtomicU64,
    bytes_client_to_remote: AtomicU64,
    bytes_remote_to_client: AtomicU64,
    spx_tcp_relay_samples: AtomicU64,
    last_spx_tcp_relay_duration_ms: AtomicU64,
    last_spx_tcp_client_to_remote_bytes: AtomicU64,
    last_spx_tcp_remote_to_client_bytes: AtomicU64,
    last_error: RwLock<Option<String>>,
    candidate_failures: RwLock<Vec<deploy::TransportCandidateFailure>>,
    last_tcp_open_error: RwLock<Option<String>>,
    last_spx_tcp_relay_close_reason: RwLock<Option<String>>,
    egress_proxy: Option<String>,
}

#[derive(Debug, Default)]
struct BridgeWorkerCounters {
    active_streams: AtomicU64,
    bytes_client_to_remote: AtomicU64,
    bytes_remote_to_client: AtomicU64,
}

#[derive(Debug, Clone)]
struct BridgeWorkerState {
    slot: usize,
    connected: bool,
    generation: u32,
    connect_attempts: u64,
    successful_connects: u64,
    failed_connects: u64,
    disconnects: u64,
    retry_count: u64,
    last_error: Option<String>,
    selected_protocol: Option<String>,
    last_successful_protocol: Option<String>,
    last_event: Option<String>,
    last_connected_at: Option<Instant>,
    last_disconnected_at: Option<Instant>,
    last_failed_at: Option<Instant>,
}

#[derive(Debug, Clone, Serialize)]
struct BridgeWorkerSnapshot {
    slot: usize,
    state: String,
    connected: bool,
    generation: u32,
    connect_attempts: u64,
    successful_connects: u64,
    failed_connects: u64,
    disconnects: u64,
    retry_count: u64,
    active_streams: u64,
    bytes_client_to_remote: u64,
    bytes_remote_to_client: u64,
    last_error: Option<String>,
    degraded_reason: Option<String>,
    selected_protocol: Option<String>,
    last_successful_protocol: Option<String>,
    last_event: Option<String>,
    last_connected_ago_secs: Option<u64>,
    last_disconnected_ago_secs: Option<u64>,
    last_failure_ago_secs: Option<u64>,
}

impl BridgeWorkerState {
    fn new(slot: usize) -> Self {
        Self {
            slot,
            connected: false,
            generation: 0,
            connect_attempts: 0,
            successful_connects: 0,
            failed_connects: 0,
            disconnects: 0,
            retry_count: 0,
            last_error: None,
            selected_protocol: None,
            last_successful_protocol: None,
            last_event: None,
            last_connected_at: None,
            last_disconnected_at: None,
            last_failed_at: None,
        }
    }

    fn connected(slot: usize, generation: u32) -> Self {
        let now = Instant::now();
        Self {
            slot,
            connected: true,
            generation,
            connect_attempts: 1,
            successful_connects: 1,
            failed_connects: 0,
            disconnects: 0,
            retry_count: 0,
            last_error: None,
            selected_protocol: Some(
                peer_transport::PeerProtocol::SshExec
                    .data_plane_label()
                    .to_string(),
            ),
            last_successful_protocol: Some(
                peer_transport::PeerProtocol::SshExec
                    .data_plane_label()
                    .to_string(),
            ),
            last_event: Some("bridge connected".to_string()),
            last_connected_at: Some(now),
            last_disconnected_at: None,
            last_failed_at: None,
        }
    }

    fn snapshot(
        &self,
        now: Instant,
        counters: Option<&BridgeWorkerCounters>,
    ) -> BridgeWorkerSnapshot {
        let state = self.health_state();
        let active_streams = counters
            .map(|counters| counters.active_streams.load(Ordering::Relaxed))
            .unwrap_or(0);
        let bytes_client_to_remote = counters
            .map(|counters| counters.bytes_client_to_remote.load(Ordering::Relaxed))
            .unwrap_or(0);
        let bytes_remote_to_client = counters
            .map(|counters| counters.bytes_remote_to_client.load(Ordering::Relaxed))
            .unwrap_or(0);
        BridgeWorkerSnapshot {
            slot: self.slot,
            state,
            connected: self.connected,
            generation: self.generation,
            connect_attempts: self.connect_attempts,
            successful_connects: self.successful_connects,
            failed_connects: self.failed_connects,
            disconnects: self.disconnects,
            retry_count: self.retry_count,
            active_streams,
            bytes_client_to_remote,
            bytes_remote_to_client,
            last_error: self.last_error.clone(),
            degraded_reason: self.degraded_reason(),
            selected_protocol: self.selected_protocol.clone(),
            last_successful_protocol: self.last_successful_protocol.clone(),
            last_event: self.last_event.clone(),
            last_connected_ago_secs: self
                .last_connected_at
                .map(|instant| now.saturating_duration_since(instant).as_secs()),
            last_disconnected_ago_secs: self
                .last_disconnected_at
                .map(|instant| now.saturating_duration_since(instant).as_secs()),
            last_failure_ago_secs: self
                .last_failed_at
                .map(|instant| now.saturating_duration_since(instant).as_secs()),
        }
    }

    fn health_state(&self) -> String {
        if self.connected {
            "connected".to_string()
        } else if self.last_event.as_deref() == Some("connecting bridge") {
            "reconnecting".to_string()
        } else if self.last_error.is_some() {
            "degraded".to_string()
        } else if self.disconnects > 0 {
            "reconnecting".to_string()
        } else {
            "idle".to_string()
        }
    }

    fn degraded_reason(&self) -> Option<String> {
        if self.connected {
            None
        } else {
            self.last_error.clone()
        }
    }
}

#[derive(Clone)]
pub struct BridgeSelection {
    pub slot: usize,
    pub handle: bridge::BridgeHandle,
}

impl SharedState {
    fn new(
        reconnect: bool,
        egress_proxy: Option<String>,
        transport_pool_size: usize,
        quic_options: peer_transport::QuicTransportOptions,
        tls_peer_auth_mode: Option<String>,
        pool_policy: Option<String>,
        workload_hint: Option<String>,
    ) -> Self {
        let transport_pool_size = transport_pool_size.max(1);
        Self {
            bridge_slots: RwLock::new(vec![None; transport_pool_size]),
            bridge_workers: RwLock::new(
                (0..transport_pool_size)
                    .map(BridgeWorkerState::new)
                    .collect(),
            ),
            bridge_worker_counters: (0..transport_pool_size)
                .map(|_| Arc::new(BridgeWorkerCounters::default()))
                .collect(),
            notify: Notify::new(),
            shutdown_notify: Notify::new(),
            shutdown: AtomicBool::new(false),
            reconnect,
            transport_pool_size,
            quic_options,
            tls_peer_auth_mode,
            pool_policy,
            workload_hint,
            next_bridge: AtomicU32::new(0),
            started: Instant::now(),
            generation: AtomicU32::new(0),
            connect_attempts: AtomicU32::new(0),
            successful_connects: AtomicU32::new(0),
            failed_connects: AtomicU32::new(0),
            active_tcp: AtomicU32::new(0),
            total_tcp: AtomicU32::new(0),
            tcp_open_attempts: AtomicU64::new(0),
            tcp_open_successes: AtomicU64::new(0),
            tcp_open_failures: AtomicU64::new(0),
            last_tcp_open_latency_ms: AtomicU64::new(0),
            ssh_direct_channel_open_samples: AtomicU64::new(0),
            last_ssh_direct_channel_open_latency_ms: AtomicU64::new(0),
            spx_peer_handshake_samples: AtomicU64::new(0),
            last_spx_peer_handshake_latency_ms: AtomicU64::new(0),
            bytes_client_to_remote: AtomicU64::new(0),
            bytes_remote_to_client: AtomicU64::new(0),
            spx_tcp_relay_samples: AtomicU64::new(0),
            last_spx_tcp_relay_duration_ms: AtomicU64::new(0),
            last_spx_tcp_client_to_remote_bytes: AtomicU64::new(0),
            last_spx_tcp_remote_to_client_bytes: AtomicU64::new(0),
            last_error: RwLock::new(None),
            candidate_failures: RwLock::new(Vec::new()),
            last_tcp_open_error: RwLock::new(None),
            last_spx_tcp_relay_close_reason: RwLock::new(None),
            egress_proxy,
        }
    }

    pub fn new_with_bridge(bridge: bridge::BridgeHandle) -> Self {
        Self::new_with_bridge_and_egress_proxy(bridge, None)
    }

    pub fn new_with_bridge_and_egress_proxy(
        bridge: bridge::BridgeHandle,
        egress_proxy: Option<String>,
    ) -> Self {
        Self {
            bridge_slots: RwLock::new(vec![Some(bridge)]),
            bridge_workers: RwLock::new(vec![BridgeWorkerState::connected(0, 1)]),
            bridge_worker_counters: vec![Arc::new(BridgeWorkerCounters::default())],
            notify: Notify::new(),
            shutdown_notify: Notify::new(),
            shutdown: AtomicBool::new(false),
            reconnect: false,
            transport_pool_size: 1,
            quic_options: peer_transport::QuicTransportOptions::default(),
            tls_peer_auth_mode: None,
            pool_policy: None,
            workload_hint: None,
            next_bridge: AtomicU32::new(0),
            started: Instant::now(),
            generation: AtomicU32::new(1),
            connect_attempts: AtomicU32::new(1),
            successful_connects: AtomicU32::new(1),
            failed_connects: AtomicU32::new(0),
            active_tcp: AtomicU32::new(0),
            total_tcp: AtomicU32::new(0),
            tcp_open_attempts: AtomicU64::new(0),
            tcp_open_successes: AtomicU64::new(0),
            tcp_open_failures: AtomicU64::new(0),
            last_tcp_open_latency_ms: AtomicU64::new(0),
            ssh_direct_channel_open_samples: AtomicU64::new(0),
            last_ssh_direct_channel_open_latency_ms: AtomicU64::new(0),
            spx_peer_handshake_samples: AtomicU64::new(0),
            last_spx_peer_handshake_latency_ms: AtomicU64::new(0),
            bytes_client_to_remote: AtomicU64::new(0),
            bytes_remote_to_client: AtomicU64::new(0),
            spx_tcp_relay_samples: AtomicU64::new(0),
            last_spx_tcp_relay_duration_ms: AtomicU64::new(0),
            last_spx_tcp_client_to_remote_bytes: AtomicU64::new(0),
            last_spx_tcp_remote_to_client_bytes: AtomicU64::new(0),
            last_error: RwLock::new(None),
            candidate_failures: RwLock::new(Vec::new()),
            last_tcp_open_error: RwLock::new(None),
            last_spx_tcp_relay_close_reason: RwLock::new(None),
            egress_proxy,
        }
    }

    async fn set_bridge(&self, slot: usize, bridge: Option<bridge::BridgeHandle>) {
        let mut slots = self.bridge_slots.write().await;
        if slot >= slots.len() {
            slots.resize_with(slot + 1, || None);
        }
        slots[slot] = bridge;
        self.notify.notify_waiters();
    }

    async fn record_bridge_attempt(&self, slot: usize) -> u32 {
        let attempt = self.connect_attempts.fetch_add(1, Ordering::Relaxed) + 1;
        let mut workers = self.bridge_workers.write().await;
        ensure_worker_slot(&mut workers, slot);
        let worker = &mut workers[slot];
        worker.connect_attempts += 1;
        worker.last_event = Some("connecting bridge".to_string());
        attempt
    }

    async fn record_bridge_connected(
        &self,
        slot: usize,
        generation: u32,
        protocol: Option<peer_transport::PeerProtocol>,
        timings: deploy::RemoteHelperTimings,
    ) {
        self.successful_connects.fetch_add(1, Ordering::Relaxed);
        if let Some(latency) = timings.ssh_direct_channel_open_latency_ms {
            self.ssh_direct_channel_open_samples
                .fetch_add(1, Ordering::Relaxed);
            self.last_ssh_direct_channel_open_latency_ms
                .store(latency, Ordering::Relaxed);
        }
        if let Some(latency) = timings.spx_peer_handshake_latency_ms {
            self.spx_peer_handshake_samples
                .fetch_add(1, Ordering::Relaxed);
            self.last_spx_peer_handshake_latency_ms
                .store(latency, Ordering::Relaxed);
        }
        self.set_last_error(None).await;
        let mut workers = self.bridge_workers.write().await;
        ensure_worker_slot(&mut workers, slot);
        let worker = &mut workers[slot];
        let protocol = protocol.map(|protocol| protocol.data_plane_label().to_string());
        worker.connected = true;
        worker.generation = generation;
        worker.successful_connects += 1;
        worker.retry_count = 0;
        worker.last_error = None;
        worker.selected_protocol = protocol.clone();
        worker.last_successful_protocol = protocol;
        worker.last_event = Some("bridge connected".to_string());
        worker.last_connected_at = Some(Instant::now());
        worker.last_failed_at = None;
        if let Some(counters) = self.bridge_worker_counters.get(slot) {
            counters.active_streams.store(0, Ordering::Relaxed);
        }
    }

    async fn record_bridge_disconnected(&self, slot: usize) {
        let mut workers = self.bridge_workers.write().await;
        ensure_worker_slot(&mut workers, slot);
        let worker = &mut workers[slot];
        worker.connected = false;
        worker.selected_protocol = None;
        worker.disconnects += 1;
        worker.last_event = Some("bridge disconnected".to_string());
        worker.last_disconnected_at = Some(Instant::now());
        if let Some(counters) = self.bridge_worker_counters.get(slot) {
            counters.active_streams.store(0, Ordering::Relaxed);
        }
    }

    async fn record_bridge_failed(&self, slot: usize, detail: String) {
        self.failed_connects.fetch_add(1, Ordering::Relaxed);
        self.set_last_error(Some(detail.clone())).await;
        let mut workers = self.bridge_workers.write().await;
        ensure_worker_slot(&mut workers, slot);
        let worker = &mut workers[slot];
        worker.connected = false;
        worker.selected_protocol = None;
        worker.failed_connects += 1;
        worker.retry_count += 1;
        worker.last_error = Some(detail);
        worker.last_event = Some("bridge connect failed".to_string());
        let now = Instant::now();
        worker.last_disconnected_at = Some(now);
        worker.last_failed_at = Some(now);
        if let Some(counters) = self.bridge_worker_counters.get(slot) {
            counters.active_streams.store(0, Ordering::Relaxed);
        }
    }

    async fn set_last_error(&self, err: Option<String>) {
        *self.last_error.write().await = err;
    }

    async fn set_candidate_failures(&self, failures: Vec<deploy::TransportCandidateFailure>) {
        *self.candidate_failures.write().await = failures;
    }

    pub async fn bridge(&self) -> Result<BridgeSelection> {
        loop {
            let available = {
                let slots = self.bridge_slots.read().await;
                let workers = self.bridge_workers.read().await;
                let mut candidates = slots
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, bridge)| {
                        let handle = bridge.clone()?;
                        let worker = workers.get(slot)?;
                        if !worker.connected {
                            return None;
                        }
                        let active_streams = self
                            .bridge_worker_counters
                            .get(slot)
                            .map(|counters| counters.active_streams.load(Ordering::Relaxed))
                            .unwrap_or(0);
                        Some((slot, active_streams, handle))
                    })
                    .collect::<Vec<_>>();
                if let Some(min_active) = candidates.iter().map(|(_, active, _)| *active).min() {
                    candidates.retain(|(_, active, _)| *active == min_active);
                }
                candidates
            };
            if !available.is_empty() {
                let index =
                    self.next_bridge.fetch_add(1, Ordering::Relaxed) as usize % available.len();
                let (slot, _, handle) = available[index].clone();
                return Ok(BridgeSelection { slot, handle });
            }
            if !self.reconnect {
                bail!("remote bridge is not connected");
            }
            tokio::select! {
                _ = self.notify.notified() => {}
                _ = self.shutdown_notified() => bail!("proxy is shutting down"),
            }
        }
    }

    async fn wait_for_initial_bridge(&self, timeout: Duration) -> Result<()> {
        let wait = async {
            loop {
                if self.active_bridge_count().await > 0 {
                    return Ok(());
                }
                if !self.reconnect
                    && self.failed_connects.load(Ordering::Relaxed) as usize
                        >= self.transport_pool_size
                {
                    let detail = self
                        .last_error
                        .read()
                        .await
                        .clone()
                        .unwrap_or_else(|| "remote bridge is not connected".to_string());
                    bail!("{detail}");
                }
                tokio::select! {
                    _ = self.notify.notified() => {}
                    _ = self.shutdown_notified() => bail!("proxy is shutting down"),
                }
            }
        };
        time::timeout(timeout, wait)
            .await
            .context("timed out waiting for initial remote bridge")?
    }

    pub fn egress_proxy(&self) -> Option<String> {
        self.egress_proxy.clone()
    }

    async fn active_bridge_count(&self) -> usize {
        self.bridge_slots
            .read()
            .await
            .iter()
            .filter(|bridge| bridge.is_some())
            .count()
    }

    async fn worker_snapshots(&self) -> Vec<BridgeWorkerSnapshot> {
        let now = Instant::now();
        self.bridge_workers
            .read()
            .await
            .iter()
            .map(|worker| {
                worker.snapshot(
                    now,
                    self.bridge_worker_counters
                        .get(worker.slot)
                        .map(Arc::as_ref),
                )
            })
            .collect()
    }

    async fn active_selected_protocol(&self) -> Option<String> {
        let workers = self.bridge_workers.read().await;
        let mut selected = None::<String>;
        for worker in workers.iter().filter(|worker| worker.connected) {
            let Some(protocol) = worker.selected_protocol.clone() else {
                continue;
            };
            match &selected {
                Some(existing) if existing != &protocol => return Some("mixed".to_string()),
                Some(_) => {}
                None => selected = Some(protocol),
            }
        }
        selected
    }

    pub fn record_tcp_open(&self) {
        self.active_tcp.fetch_add(1, Ordering::Relaxed);
        self.total_tcp.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_tcp_close(&self) {
        self.active_tcp
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(1))
            .ok();
    }

    pub fn record_worker_tcp_open(&self, slot: usize) {
        if let Some(counters) = self.bridge_worker_counters.get(slot) {
            counters.active_streams.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_worker_tcp_close(&self, slot: usize) {
        if let Some(counters) = self.bridge_worker_counters.get(slot) {
            counters
                .active_streams
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(1))
                .ok();
        }
    }

    pub fn record_tcp_open_attempt(&self) {
        self.tcp_open_attempts.fetch_add(1, Ordering::Relaxed);
    }

    pub async fn record_tcp_open_success(&self) {
        self.tcp_open_successes.fetch_add(1, Ordering::Relaxed);
        *self.last_tcp_open_error.write().await = None;
    }

    pub async fn record_tcp_open_failure(&self, err: impl Into<String>) {
        self.tcp_open_failures.fetch_add(1, Ordering::Relaxed);
        *self.last_tcp_open_error.write().await = Some(err.into());
    }

    pub fn record_tcp_open_latency(&self, latency: Duration) {
        self.last_tcp_open_latency_ms
            .store(duration_millis(latency), Ordering::Relaxed);
    }

    pub fn record_client_to_remote_bytes(&self, bytes: usize) {
        self.bytes_client_to_remote
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    pub fn record_remote_to_client_bytes(&self, bytes: usize) {
        self.bytes_remote_to_client
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    pub fn record_worker_client_to_remote_bytes(&self, slot: usize, bytes: usize) {
        if let Some(counters) = self.bridge_worker_counters.get(slot) {
            counters
                .bytes_client_to_remote
                .fetch_add(bytes as u64, Ordering::Relaxed);
        }
    }

    pub fn record_worker_remote_to_client_bytes(&self, slot: usize, bytes: usize) {
        if let Some(counters) = self.bridge_worker_counters.get(slot) {
            counters
                .bytes_remote_to_client
                .fetch_add(bytes as u64, Ordering::Relaxed);
        }
    }

    pub async fn record_spx_tcp_relay(
        &self,
        duration: Duration,
        client_to_remote_bytes: usize,
        remote_to_client_bytes: usize,
        close_reason: impl Into<String>,
    ) {
        self.spx_tcp_relay_samples.fetch_add(1, Ordering::Relaxed);
        self.last_spx_tcp_relay_duration_ms
            .store(duration_millis(duration), Ordering::Relaxed);
        self.last_spx_tcp_client_to_remote_bytes
            .store(client_to_remote_bytes as u64, Ordering::Relaxed);
        self.last_spx_tcp_remote_to_client_bytes
            .store(remote_to_client_bytes as u64, Ordering::Relaxed);
        *self.last_spx_tcp_relay_close_reason.write().await = Some(close_reason.into());
    }

    async fn bridge_metrics_snapshot(&self) -> bridge::BridgeMetricsSnapshot {
        let mut aggregate = bridge::BridgeMetricsSnapshot::default();
        for bridge in self
            .bridge_slots
            .read()
            .await
            .iter()
            .filter_map(Clone::clone)
        {
            aggregate.merge(&bridge.metrics_snapshot());
        }
        aggregate
    }

    fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.shutdown_notify.notify_waiters();
        self.notify.notify_waiters();
    }

    async fn shutdown_notified(&self) {
        if self.shutdown.load(Ordering::Relaxed) {
            return;
        }
        self.shutdown_notify.notified().await;
    }

    pub async fn status_value(&self) -> serde_json::Value {
        let active_bridges = self.active_bridge_count().await;
        let workers = self.worker_snapshots().await;
        let selected_protocol = self.active_selected_protocol().await;
        let bridge_metrics = self.bridge_metrics_snapshot().await;
        let healthy_workers = workers
            .iter()
            .filter(|worker| worker.state == "connected")
            .count();
        let degraded_workers = workers
            .iter()
            .filter(|worker| worker.state == "degraded")
            .count();
        let reconnecting_workers = workers
            .iter()
            .filter(|worker| worker.state == "reconnecting")
            .count();
        let pool_degraded_reason =
            if healthy_workers == 0 && (degraded_workers > 0 || reconnecting_workers > 0) {
                Some("all_workers_unavailable")
            } else if degraded_workers > 0 || reconnecting_workers > 0 {
                Some("partial_pool_degraded")
            } else {
                None
            };
        let mut status = serde_json::Map::new();

        status.insert("connected".to_string(), (active_bridges > 0).into());
        status.insert("reconnect".to_string(), self.reconnect.into());
        status.insert(
            "shutting_down".to_string(),
            self.shutdown.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "transport_pool_size".to_string(),
            self.transport_pool_size.into(),
        );
        status.insert(
            "pool_policy".to_string(),
            serde_json::to_value(self.pool_policy.clone()).expect("pool policy serializable"),
        );
        status.insert(
            "workload_hint".to_string(),
            serde_json::to_value(self.workload_hint.clone()).expect("workload hint serializable"),
        );
        status.insert("active_bridges".to_string(), active_bridges.into());
        status.insert("healthy_workers".to_string(), healthy_workers.into());
        status.insert("degraded_workers".to_string(), degraded_workers.into());
        status.insert(
            "reconnecting_workers".to_string(),
            reconnecting_workers.into(),
        );
        status.insert(
            "pool_degraded_reason".to_string(),
            serde_json::to_value(pool_degraded_reason).expect("pool degraded reason serializable"),
        );
        status.insert(
            "selected_protocol".to_string(),
            serde_json::to_value(selected_protocol.clone())
                .expect("selected protocol serializable"),
        );
        status.insert(
            "tls_peer_auth_mode".to_string(),
            serde_json::to_value(self.tls_peer_auth_mode.clone())
                .expect("tls peer auth mode serializable"),
        );
        status.insert(
            "workers".to_string(),
            serde_json::to_value(workers).expect("workers serializable"),
        );
        status.insert(
            "uptime_secs".to_string(),
            self.started.elapsed().as_secs().into(),
        );
        status.insert(
            "generation".to_string(),
            self.generation.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "connect_attempts".to_string(),
            self.connect_attempts.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "successful_connects".to_string(),
            self.successful_connects.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "failed_connects".to_string(),
            self.failed_connects.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "active_tcp".to_string(),
            self.active_tcp.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "total_tcp".to_string(),
            self.total_tcp.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "tcp_open_attempts".to_string(),
            self.tcp_open_attempts.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "tcp_open_successes".to_string(),
            self.tcp_open_successes.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "tcp_open_failures".to_string(),
            self.tcp_open_failures.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "last_tcp_open_latency_ms".to_string(),
            serde_json::to_value(last_sampled_u64(
                self.tcp_open_attempts.load(Ordering::Relaxed),
                self.last_tcp_open_latency_ms.load(Ordering::Relaxed),
            ))
            .expect("tcp open latency serializable"),
        );
        status.insert(
            "ssh_direct_channel_open_samples".to_string(),
            self.ssh_direct_channel_open_samples
                .load(Ordering::Relaxed)
                .into(),
        );
        status.insert(
            "last_ssh_direct_channel_open_latency_ms".to_string(),
            serde_json::to_value(last_sampled_u64(
                self.ssh_direct_channel_open_samples.load(Ordering::Relaxed),
                self.last_ssh_direct_channel_open_latency_ms
                    .load(Ordering::Relaxed),
            ))
            .expect("ssh direct channel open latency serializable"),
        );
        status.insert(
            "spx_peer_handshake_samples".to_string(),
            self.spx_peer_handshake_samples
                .load(Ordering::Relaxed)
                .into(),
        );
        status.insert(
            "last_spx_peer_handshake_latency_ms".to_string(),
            serde_json::to_value(last_sampled_u64(
                self.spx_peer_handshake_samples.load(Ordering::Relaxed),
                self.last_spx_peer_handshake_latency_ms
                    .load(Ordering::Relaxed),
            ))
            .expect("spx peer handshake latency serializable"),
        );
        status.insert(
            "bytes_client_to_remote".to_string(),
            self.bytes_client_to_remote.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "bytes_remote_to_client".to_string(),
            self.bytes_remote_to_client.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "spx_tcp_relay_samples".to_string(),
            self.spx_tcp_relay_samples.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "last_spx_tcp_relay_duration_ms".to_string(),
            serde_json::to_value(last_sampled_u64(
                self.spx_tcp_relay_samples.load(Ordering::Relaxed),
                self.last_spx_tcp_relay_duration_ms.load(Ordering::Relaxed),
            ))
            .expect("spx relay duration serializable"),
        );
        status.insert(
            "last_spx_tcp_client_to_remote_bytes".to_string(),
            self.last_spx_tcp_client_to_remote_bytes
                .load(Ordering::Relaxed)
                .into(),
        );
        status.insert(
            "last_spx_tcp_remote_to_client_bytes".to_string(),
            self.last_spx_tcp_remote_to_client_bytes
                .load(Ordering::Relaxed)
                .into(),
        );
        status.insert(
            "last_spx_tcp_relay_close_reason".to_string(),
            serde_json::to_value(self.last_spx_tcp_relay_close_reason.read().await.clone())
                .expect("spx relay close reason serializable"),
        );
        status.insert(
            "link".to_string(),
            serde_json::json!({
                "health": {
                    "selected_protocol": selected_protocol,
                    "active_connections": active_bridges,
                    "active_streams": self.active_tcp.load(Ordering::Relaxed),
                    "active_channels": self.active_tcp.load(Ordering::Relaxed),
                    "pool_size": self.transport_pool_size,
                    "pool_policy": self.pool_policy.clone(),
                    "pool_workload_hint": self.workload_hint.clone(),
                    "open_attempts": self.tcp_open_attempts.load(Ordering::Relaxed),
                    "open_successes": self.tcp_open_successes.load(Ordering::Relaxed),
                    "open_failures": self.tcp_open_failures.load(Ordering::Relaxed),
                    "open_latency_ms": last_sampled_u64(
                        self.tcp_open_attempts.load(Ordering::Relaxed),
                        self.last_tcp_open_latency_ms.load(Ordering::Relaxed),
                    ),
                    "bytes_client_to_remote": self.bytes_client_to_remote.load(Ordering::Relaxed),
                    "bytes_remote_to_client": self.bytes_remote_to_client.load(Ordering::Relaxed),
                    "first_byte_latency_ms": serde_json::Value::Null,
                    "first_byte_samples": 0,
                    "max_first_byte_latency_ms": serde_json::Value::Null,
                    "last_close_reason": self.last_spx_tcp_relay_close_reason.read().await.clone(),
                    "degraded_reason": pool_degraded_reason,
                    "healthy_workers": healthy_workers,
                    "degraded_workers": degraded_workers,
                    "reconnecting_workers": reconnecting_workers,
                    "control_health": if active_bridges > 0 && pool_degraded_reason.is_none() {
                        "healthy"
                    } else if active_bridges > 0 {
                        "degraded"
                    } else if self.reconnect {
                        "reconnecting"
                    } else {
                        "disconnected"
                    },
                    "connected": active_bridges > 0,
                }
            }),
        );
        status.insert(
            "spx_frame_write_batches".to_string(),
            bridge_metrics.frame_write_batches.into(),
        );
        status.insert(
            "spx_frame_write_flushes".to_string(),
            bridge_metrics.frame_write_flushes.into(),
        );
        status.insert(
            "spx_frame_write_frames".to_string(),
            bridge_metrics.frame_write_frames.into(),
        );
        status.insert(
            "spx_frame_write_data_frames".to_string(),
            bridge_metrics.frame_write_data_frames.into(),
        );
        status.insert(
            "spx_frame_write_data_bytes".to_string(),
            bridge_metrics.frame_write_data_bytes.into(),
        );
        status.insert(
            "spx_frame_write_vectored_writes".to_string(),
            bridge_metrics.frame_write_vectored_writes.into(),
        );
        status.insert(
            "spx_frame_write_failures".to_string(),
            bridge_metrics.frame_write_failures.into(),
        );
        status.insert(
            "spx_frame_read_frames".to_string(),
            bridge_metrics.frame_read_frames.into(),
        );
        status.insert(
            "spx_frame_read_data_frames".to_string(),
            bridge_metrics.frame_read_data_frames.into(),
        );
        status.insert(
            "spx_frame_read_data_bytes".to_string(),
            bridge_metrics.frame_read_data_bytes.into(),
        );
        status.insert(
            "spx_tcp_stream_backpressure_timeouts".to_string(),
            bridge_metrics.tcp_stream_backpressure_timeouts.into(),
        );
        status.insert(
            "spx_udp_assoc_backpressure_timeouts".to_string(),
            bridge_metrics.udp_assoc_backpressure_timeouts.into(),
        );
        status.insert(
            "read_buffer_size".to_string(),
            protocol::TCP_DATA_CHUNK.into(),
        );
        status.insert(
            "write_batch_limit".to_string(),
            protocol::FRAME_WRITE_BATCH_LIMIT.into(),
        );
        status.insert(
            "frame_channel_capacity".to_string(),
            protocol::FRAME_CHANNEL_CAPACITY.into(),
        );
        status.insert(
            "quic_receive_window".to_string(),
            self.quic_options.receive_window.into(),
        );
        status.insert(
            "quic_stream_receive_window".to_string(),
            self.quic_options.stream_receive_window.into(),
        );
        status.insert(
            "quic_max_bidi_streams".to_string(),
            self.quic_options.max_bidi_streams.into(),
        );
        status.insert(
            "quic_keep_alive_interval_secs".to_string(),
            self.quic_options.keep_alive_interval_secs.into(),
        );
        status.insert(
            "quic_idle_timeout_secs".to_string(),
            self.quic_options.idle_timeout_secs.into(),
        );
        status.insert(
            "quic_runtime".to_string(),
            serde_json::to_value(peer_transport::quic_runtime_diagnostics(self.quic_options))
                .expect("quic runtime diagnostics serializable"),
        );
        status.insert(
            "quic_udp_runtime".to_string(),
            peer_transport::QUIC_UDP_RUNTIME.into(),
        );
        status.insert("quic_udp_gso".to_string(), serde_json::Value::Null);
        status.insert(
            "quic_udp_gso_source".to_string(),
            peer_transport::QUIC_UDP_GSO_SOURCE.into(),
        );
        status.insert(
            "quic_packetization".to_string(),
            peer_transport::QUIC_PACKETIZATION.into(),
        );
        status.insert(
            "last_error".to_string(),
            serde_json::to_value(self.last_error.read().await.clone())
                .expect("last error serializable"),
        );
        status.insert(
            "candidate_failures".to_string(),
            serde_json::to_value(self.candidate_failures.read().await.clone())
                .expect("candidate failures serializable"),
        );
        status.insert(
            "last_tcp_open_error".to_string(),
            serde_json::to_value(self.last_tcp_open_error.read().await.clone())
                .expect("last tcp open error serializable"),
        );

        serde_json::Value::Object(status)
    }

    async fn status_json(&self) -> Result<String> {
        let status = self.status_value().await;
        Ok(format!("{}\n", serde_json::to_string_pretty(&status)?))
    }
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn last_sampled_u64(sample_count: u64, value: u64) -> Option<u64> {
    (sample_count > 0).then_some(value)
}

fn jittered_backoff(base: Duration, max_delay: Duration) -> Duration {
    let base_ms = duration_millis(base).max(1);
    let max_ms = duration_millis(max_delay).max(base_ms);
    let jitter_range_ms = (base_ms / 4).max(1);
    let seed = random_u64().unwrap_or_else(|| base_ms.rotate_left(13) ^ max_ms);
    let offset_ms = seed % (jitter_range_ms + 1);
    let jittered_ms = if seed & 1 == 0 {
        base_ms.saturating_add(offset_ms).min(max_ms)
    } else {
        base_ms.saturating_sub(offset_ms).max(1)
    };
    Duration::from_millis(jittered_ms)
}

fn random_u64() -> Option<u64> {
    let mut bytes = [0_u8; 8];
    getrandom::fill(&mut bytes).ok()?;
    Some(u64::from_le_bytes(bytes))
}

fn ensure_worker_slot(workers: &mut Vec<BridgeWorkerState>, slot: usize) {
    while workers.len() <= slot {
        let next = workers.len();
        workers.push(BridgeWorkerState::new(next));
    }
}

async fn run_control_server(addr: SocketAddr, state: Arc<SharedState>) -> Result<()> {
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind control listener {addr}"))?;
    info!(%addr, "control listener ready");
    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, peer) = accept?;
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_control(stream, state).await {
                        warn!(%peer, error = %err, "control request failed");
                    }
                });
            }
            _ = state.shutdown_notified() => break,
        }
    }
    Ok(())
}

async fn handle_control(stream: TcpStream, state: Arc<SharedState>) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut command = String::new();
    reader.read_line(&mut command).await?;
    match command.trim().to_ascii_lowercase().as_str() {
        "status" | "" => {
            writer
                .write_all(state.status_json().await?.as_bytes())
                .await?
        }
        "shutdown" => {
            state.request_shutdown();
            writer
                .write_all(b"{\"ok\":true,\"message\":\"shutdown requested\"}\n")
                .await?;
        }
        other => {
            let response = serde_json::json!({
                "ok": false,
                "error": format!("unknown command {other:?}; expected status or shutdown")
            });
            writer
                .write_all(format!("{}\n", serde_json::to_string_pretty(&response)?).as_bytes())
                .await?;
        }
    }
    writer.shutdown().await.ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn shared_status_exposes_normalized_link_health() {
        let state = SharedState::new(
            false,
            None,
            1,
            peer_transport::QuicTransportOptions::default(),
            None,
            None,
            None,
        );

        let status = state.status_value().await;

        assert_eq!(status["link"]["health"]["active_connections"], 0);
        assert_eq!(status["link"]["health"]["active_streams"], 0);
        assert_eq!(status["link"]["health"]["active_channels"], 0);
        assert_eq!(status["link"]["health"]["open_attempts"], 0);
        assert_eq!(status["link"]["health"]["open_successes"], 0);
        assert_eq!(status["link"]["health"]["open_failures"], 0);
        assert_eq!(
            status["link"]["health"]["open_latency_ms"],
            serde_json::Value::Null
        );
        assert_eq!(
            status["link"]["health"]["first_byte_latency_ms"],
            serde_json::Value::Null
        );
        assert_eq!(status["link"]["health"]["first_byte_samples"], 0);
        assert_eq!(
            status["link"]["health"]["last_close_reason"],
            serde_json::Value::Null
        );
        assert_eq!(status["link"]["health"]["control_health"], "disconnected");
        assert_eq!(status["spx_frame_write_vectored_writes"], 0);
    }

    #[tokio::test]
    async fn bridge_selection_skips_busy_workers_and_reports_partial_health() {
        let state = SharedState::new(
            true,
            None,
            2,
            peer_transport::QuicTransportOptions::default(),
            None,
            None,
            None,
        );
        let (a0, b0) = duplex(64);
        let (a1, b1) = duplex(64);
        let bridge0 = bridge::connect_io(a0, b0).await.unwrap();
        let bridge1 = bridge::connect_io(a1, b1).await.unwrap();
        state.set_bridge(0, Some(bridge0.handle.clone())).await;
        state.set_bridge(1, Some(bridge1.handle.clone())).await;
        state
            .record_bridge_connected(
                0,
                1,
                Some(peer_transport::PeerProtocol::SshExec),
                Default::default(),
            )
            .await;
        state
            .record_bridge_connected(
                1,
                2,
                Some(peer_transport::PeerProtocol::SshExec),
                Default::default(),
            )
            .await;
        state.record_worker_tcp_open(0);
        state.record_worker_tcp_open(0);
        state.record_worker_tcp_open(1);

        let bridge = state.bridge().await.expect("bridge available");

        assert_eq!(bridge.slot, 1);

        let status = state.status_value().await;
        assert_eq!(status["healthy_workers"], 2);
        assert_eq!(status["degraded_workers"], 0);
        assert_eq!(status["reconnecting_workers"], 0);
        assert_eq!(status["pool_degraded_reason"], serde_json::Value::Null);
        assert_eq!(
            status["link"]["health"]["selected_protocol"],
            "ssh-exec-spx"
        );
        assert_eq!(status["link"]["health"]["active_connections"], 2);
    }
}
