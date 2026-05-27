use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use tokio::{
    net::TcpListener,
    sync::{Notify, RwLock},
    time,
};
use tracing::{debug, error, info, warn};

use crate::{
    bridge, cli, config, control_socket, deploy, peer_transport, quic_native, socks, ssh_native,
};

mod bridge_manager;
mod listener;
mod status;
use status::{BridgeWorkerCounters, BridgeWorkerState, ensure_worker_slot};

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
        bridge_manager::run(manager_args, manager_state).await;
    });
    if args.no_reconnect {
        state
            .wait_for_initial_bridge(Duration::from_secs(args.connect_timeout_secs.max(1)))
            .await?;
    }

    if let Some(addr) = args.control_listen {
        let control_state = state.clone();
        tokio::spawn(async move {
            if let Err(err) = listener::run_control_server(addr, control_state).await {
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
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
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
