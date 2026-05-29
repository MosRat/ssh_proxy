use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{Mutex, Notify, RwLock},
    time,
};
use tracing::{debug, info, warn};

use super::metrics::{duration_millis, record_latency_sample, update_max};
use super::pool::quic_worker_score;
use super::runtime_config::route_id;
use super::runtime_status;

mod connection;
mod control_loop;
mod metrics_snapshot;
mod status;
mod stream;

use connection::connect_worker;
use control_loop::run_control_loop;

const CONTROL_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const CONTROL_KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(45);
pub const QUIC_NATIVE_COPY_BUFFER_SIZE: usize = crate::protocol::TCP_DATA_CHUNK;
pub const QUIC_NATIVE_BACKPRESSURE_TIMEOUT: Duration = Duration::from_secs(120);
pub const QUIC_NATIVE_FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(15);

use crate::{
    cli,
    quic_native::{
        flow::write_flow_header_with_buffer,
        stream_header::{StreamHeader, StreamTarget},
    },
    quic_stream, socks,
};

pub struct State {
    args: cli::ProxyArgs,
    workers: Vec<Arc<ConnectionWorker>>,
    route_id: String,
    started: Instant,
    shutdown: AtomicBool,
    shutdown_notify: Notify,
    next_worker: AtomicU64,
    next_stream_id: AtomicU64,
    tcp_open_attempts: AtomicU64,
    tcp_open_successes: AtomicU64,
    tcp_open_failures: AtomicU64,
    last_tcp_open_latency_ms: AtomicU64,
    active_quic_flows: AtomicU64,
    quic_stream_open_samples: AtomicU64,
    last_quic_stream_open_latency_ms: AtomicU64,
    max_quic_stream_open_latency_ms: AtomicU64,
    quic_stream_open_failures: AtomicU64,
    quic_header_write_samples: AtomicU64,
    last_quic_header_write_latency_ms: AtomicU64,
    max_quic_header_write_latency_ms: AtomicU64,
    quic_header_write_failures: AtomicU64,
    quic_backpressure_timeouts: AtomicU64,
    quic_flow_graceful_closes: AtomicU64,
    quic_flow_resets: AtomicU64,
    quic_flow_first_byte_samples: AtomicU64,
    last_quic_flow_first_byte_latency_ms: AtomicU64,
    max_quic_flow_first_byte_latency_ms: AtomicU64,
    quic_copy_duration_samples: AtomicU64,
    last_quic_copy_duration_ms: AtomicU64,
    max_quic_copy_duration_ms: AtomicU64,
    quic_copy_failures: AtomicU64,
    last_quic_copy_client_to_remote_bytes: AtomicU64,
    last_quic_copy_remote_to_client_bytes: AtomicU64,
    max_quic_copy_client_to_remote_bytes: AtomicU64,
    max_quic_copy_remote_to_client_bytes: AtomicU64,
    active_tcp: AtomicU64,
    total_tcp: AtomicU64,
    bytes_client_to_remote: AtomicU64,
    bytes_remote_to_client: AtomicU64,
    control_degraded: AtomicBool,
    control_pings_sent: AtomicU64,
    control_pongs_received: AtomicU64,
    last_control_pong_latency_ms: AtomicU64,
    last_control_error: Mutex<Option<String>>,
    last_quic_flow_close_reason: Mutex<Option<String>>,
    last_error: Mutex<Option<String>>,
}

struct ConnectionWorker {
    id: usize,
    connection: quinn::Connection,
    started: Instant,
    active_quic_flows: AtomicU64,
    opened_quic_flows: AtomicU64,
    stream_open_failures: AtomicU64,
    last_stream_open_latency_ms: AtomicU64,
    bytes_client_to_remote: AtomicU64,
    bytes_remote_to_client: AtomicU64,
    quic_flow_graceful_closes: AtomicU64,
    quic_flow_resets: AtomicU64,
    quic_backpressure_timeouts: AtomicU64,
    control_degraded: AtomicBool,
    control_pings_sent: AtomicU64,
    control_pongs_received: AtomicU64,
    last_control_pong_latency_ms: AtomicU64,
    last_control_error: Mutex<Option<String>>,
}

pub struct Stream {
    inner: quic_stream::QuicBiStream,
    state: Arc<State>,
    worker: Arc<ConnectionWorker>,
    closed: AtomicBool,
    opened_at: Instant,
    first_byte_recorded: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct StateSlot {
    current: RwLock<Option<Arc<State>>>,
    last_error: Mutex<Option<String>>,
}

impl StateSlot {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub async fn set_current(&self, state: Arc<State>) {
        *self.current.write().await = Some(state);
        *self.last_error.lock().await = None;
    }

    pub async fn clear_current(&self, state: &Arc<State>, err: Option<String>) {
        let mut current = self.current.write().await;
        if current
            .as_ref()
            .is_some_and(|existing| Arc::ptr_eq(existing, state))
        {
            *current = None;
        }
        if let Some(err) = err {
            *self.last_error.lock().await = Some(err);
        }
    }

    pub async fn status_value(&self) -> serde_json::Value {
        if let Some(state) = self.current.read().await.clone() {
            return state.status_value().await;
        }
        let last_error = self.last_error.lock().await.clone();
        runtime_status::disconnected_status_value(
            last_error,
            CONTROL_KEEPALIVE_INTERVAL.as_secs(),
            CONTROL_KEEPALIVE_TIMEOUT.as_secs(),
            QUIC_NATIVE_COPY_BUFFER_SIZE,
            QUIC_NATIVE_FIRST_BYTE_TIMEOUT.as_secs(),
            QUIC_NATIVE_BACKPRESSURE_TIMEOUT.as_secs(),
        )
    }
}

impl ConnectionWorker {
    fn new(id: usize, connection: quinn::Connection) -> Self {
        Self {
            id,
            connection,
            started: Instant::now(),
            active_quic_flows: AtomicU64::new(0),
            opened_quic_flows: AtomicU64::new(0),
            stream_open_failures: AtomicU64::new(0),
            last_stream_open_latency_ms: AtomicU64::new(0),
            bytes_client_to_remote: AtomicU64::new(0),
            bytes_remote_to_client: AtomicU64::new(0),
            quic_flow_graceful_closes: AtomicU64::new(0),
            quic_flow_resets: AtomicU64::new(0),
            quic_backpressure_timeouts: AtomicU64::new(0),
            control_degraded: AtomicBool::new(false),
            control_pings_sent: AtomicU64::new(0),
            control_pongs_received: AtomicU64::new(0),
            last_control_pong_latency_ms: AtomicU64::new(0),
            last_control_error: Mutex::new(None),
        }
    }

    fn score(&self) -> u64 {
        quic_worker_score(
            self.active_quic_flows.load(Ordering::Relaxed),
            self.stream_open_failures.load(Ordering::Relaxed),
            self.last_stream_open_latency_ms.load(Ordering::Relaxed),
            self.quic_backpressure_timeouts.load(Ordering::Relaxed),
            self.quic_flow_resets.load(Ordering::Relaxed),
            self.control_degraded.load(Ordering::Relaxed),
        )
    }

    fn record_open_success(&self, latency: Duration) {
        self.opened_quic_flows.fetch_add(1, Ordering::Relaxed);
        self.last_stream_open_latency_ms
            .store(duration_millis(latency), Ordering::Relaxed);
    }

    fn record_open_failure(&self) {
        self.stream_open_failures.fetch_add(1, Ordering::Relaxed);
    }

    fn record_flow_opened(&self) {
        self.active_quic_flows.fetch_add(1, Ordering::Relaxed);
    }

    fn record_flow_closed(&self, reset: bool) {
        self.active_quic_flows
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_sub(1)
            })
            .ok();
        if reset {
            self.quic_flow_resets.fetch_add(1, Ordering::Relaxed);
        } else {
            self.quic_flow_graceful_closes
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_backpressure_timeout(&self) {
        self.quic_backpressure_timeouts
            .fetch_add(1, Ordering::Relaxed);
    }

    async fn mark_control_degraded(&self, err: impl Into<String>) {
        self.control_degraded.store(true, Ordering::Relaxed);
        *self.last_control_error.lock().await = Some(err.into());
    }
}

impl State {
    pub async fn connect(args: cli::ProxyArgs) -> Result<Arc<Self>> {
        let route_id = route_id(&args);
        let pool_size = args.transport_pool_size.max(1);
        let mut connected = Vec::with_capacity(pool_size);
        let mut negotiated_route_id = route_id.clone();
        let mut last_error = None;
        for id in 0..pool_size {
            match connect_worker(&args, &route_id, id).await {
                Ok(connected_worker) => {
                    negotiated_route_id = connected_worker.route_id.clone();
                    connected.push(connected_worker);
                }
                Err(err) => {
                    last_error = Some(err.to_string());
                    if connected.is_empty() {
                        return Err(err);
                    }
                    warn!(
                        worker_id = id,
                        error = %err,
                        "quic-native connection pool worker failed during startup; continuing with partial pool"
                    );
                    break;
                }
            }
        }
        let state = Arc::new(Self {
            args,
            workers: connected
                .iter()
                .map(|worker| worker.worker.clone())
                .collect(),
            route_id: negotiated_route_id.clone(),
            started: Instant::now(),
            shutdown: AtomicBool::new(false),
            shutdown_notify: Notify::new(),
            next_worker: AtomicU64::new(0),
            next_stream_id: AtomicU64::new(1),
            tcp_open_attempts: AtomicU64::new(0),
            tcp_open_successes: AtomicU64::new(0),
            tcp_open_failures: AtomicU64::new(0),
            last_tcp_open_latency_ms: AtomicU64::new(0),
            active_quic_flows: AtomicU64::new(0),
            quic_stream_open_samples: AtomicU64::new(0),
            last_quic_stream_open_latency_ms: AtomicU64::new(0),
            max_quic_stream_open_latency_ms: AtomicU64::new(0),
            quic_stream_open_failures: AtomicU64::new(0),
            quic_header_write_samples: AtomicU64::new(0),
            last_quic_header_write_latency_ms: AtomicU64::new(0),
            max_quic_header_write_latency_ms: AtomicU64::new(0),
            quic_header_write_failures: AtomicU64::new(0),
            quic_backpressure_timeouts: AtomicU64::new(0),
            quic_flow_graceful_closes: AtomicU64::new(0),
            quic_flow_resets: AtomicU64::new(0),
            quic_flow_first_byte_samples: AtomicU64::new(0),
            last_quic_flow_first_byte_latency_ms: AtomicU64::new(0),
            max_quic_flow_first_byte_latency_ms: AtomicU64::new(0),
            quic_copy_duration_samples: AtomicU64::new(0),
            last_quic_copy_duration_ms: AtomicU64::new(0),
            max_quic_copy_duration_ms: AtomicU64::new(0),
            quic_copy_failures: AtomicU64::new(0),
            last_quic_copy_client_to_remote_bytes: AtomicU64::new(0),
            last_quic_copy_remote_to_client_bytes: AtomicU64::new(0),
            max_quic_copy_client_to_remote_bytes: AtomicU64::new(0),
            max_quic_copy_remote_to_client_bytes: AtomicU64::new(0),
            active_tcp: AtomicU64::new(0),
            total_tcp: AtomicU64::new(0),
            bytes_client_to_remote: AtomicU64::new(0),
            bytes_remote_to_client: AtomicU64::new(0),
            control_degraded: AtomicBool::new(false),
            control_pings_sent: AtomicU64::new(0),
            control_pongs_received: AtomicU64::new(0),
            last_control_pong_latency_ms: AtomicU64::new(0),
            last_control_error: Mutex::new(None),
            last_quic_flow_close_reason: Mutex::new(None),
            last_error: Mutex::new(last_error),
        });
        for connected_worker in connected {
            let control_state = state.clone();
            let worker = connected_worker.worker.clone();
            tokio::spawn(async move {
                if let Err(err) =
                    run_control_loop(connected_worker.control, control_state, worker).await
                {
                    warn!(error = %err, "quic-native control loop stopped");
                }
            });
        }
        info!(
            remote_quic = ?state.args.remote_quic,
            pool_size = state.workers.len(),
            route_id = %negotiated_route_id,
            "connected to remote QUIC-native transport"
        );
        Ok(state)
    }

    pub async fn open_stream(
        self: &Arc<Self>,
        target: StreamTarget,
        egress_proxy: Option<String>,
    ) -> Result<Stream> {
        let worker = self.select_worker().await?;
        let started = Instant::now();
        let opened = match time::timeout(
            Duration::from_secs(self.args.connect_timeout_secs.max(1)),
            worker.connection.open_bi(),
        )
        .await
        {
            Ok(Ok(streams)) => Ok(streams),
            Ok(Err(err)) => Err(anyhow!(err)),
            Err(_) => Err(anyhow!(
                "quic-native stream open timed out after {}s",
                self.args.connect_timeout_secs.max(1)
            )),
        };

        let (send, recv) = match opened {
            Ok(streams) => {
                let open_latency = started.elapsed();
                worker.record_open_success(open_latency);
                self.record_quic_stream_open_latency(open_latency);
                debug!(
                    worker_id = worker.id,
                    stream_open_latency_ms = duration_millis(open_latency),
                    "opened QUIC-native bidi stream"
                );
                streams
            }
            Err(err) => {
                worker.record_open_failure();
                self.quic_stream_open_failures
                    .fetch_add(1, Ordering::Relaxed);
                *self.last_error.lock().await = Some(err.to_string());
                debug!(
                    worker_id = worker.id,
                    error = %err,
                    "failed to open QUIC-native bidi stream"
                );
                return Err(err);
            }
        };

        worker.record_flow_opened();
        self.active_quic_flows.fetch_add(1, Ordering::Relaxed);
        let mut inner =
            quic_stream::QuicBiStream::with_connection(send, recv, worker.connection.clone());
        let header = StreamHeader {
            route_id: self.route_id.clone(),
            stream_id: self.next_stream_id.fetch_add(1, Ordering::Relaxed),
            target,
            egress_proxy,
            flags: 0,
        };
        let mut header_buffer = Vec::with_capacity(header.encoded_capacity_hint());
        let header_started = Instant::now();
        let header_write = time::timeout(
            QUIC_NATIVE_BACKPRESSURE_TIMEOUT,
            write_flow_header_with_buffer(&mut inner, &header, &mut header_buffer),
        )
        .await;
        if let Err(err) = match header_write {
            Ok(result) => result,
            Err(_) => {
                worker.record_backpressure_timeout();
                self.record_quic_backpressure_timeout();
                Err(anyhow!(
                    "QUIC-native flow header write timed out after {}s",
                    QUIC_NATIVE_BACKPRESSURE_TIMEOUT.as_secs()
                ))
            }
        } {
            worker.record_flow_closed(true);
            self.active_quic_flows.fetch_sub(1, Ordering::Relaxed);
            self.quic_header_write_failures
                .fetch_add(1, Ordering::Relaxed);
            *self.last_error.lock().await = Some(err.to_string());
            debug!(
                worker_id = worker.id,
                stream_id = header.stream_id,
                error = %err,
                "failed to write QUIC-native flow header"
            );
            return Err(err);
        }
        let header_write_latency = header_started.elapsed();
        self.record_quic_header_write_latency(header_write_latency);
        debug!(
            worker_id = worker.id,
            stream_id = header.stream_id,
            header_write_latency_ms = duration_millis(header_write_latency),
            "wrote QUIC-native flow header"
        );
        *self.last_error.lock().await = None;
        Ok(Stream {
            inner,
            state: self.clone(),
            worker,
            closed: AtomicBool::new(false),
            opened_at: Instant::now(),
            first_byte_recorded: Arc::new(AtomicBool::new(false)),
        })
    }

    async fn select_worker(&self) -> Result<Arc<ConnectionWorker>> {
        let Some(worker) = self.workers.first().cloned() else {
            return Err(anyhow!("quic-native connection pool is empty"));
        };
        if self.workers.len() == 1 {
            return Ok(worker);
        }
        let next = self.next_worker.fetch_add(1, Ordering::Relaxed) as usize;
        let mut best = None::<(u64, Arc<ConnectionWorker>)>;
        for offset in 0..self.workers.len() {
            let candidate = self.workers[(next + offset) % self.workers.len()].clone();
            let score = candidate.score();
            match &best {
                Some((best_score, _)) if *best_score <= score => {}
                _ => best = Some((score, candidate)),
            }
        }
        Ok(best.map(|(_, worker)| worker).unwrap_or(worker))
    }

    pub fn egress_proxy(&self) -> Option<String> {
        self.args.egress_proxy.clone()
    }

    pub fn record_tcp_open(&self) {
        self.total_tcp.fetch_add(1, Ordering::Relaxed);
        self.active_tcp.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_tcp_close(&self) {
        self.active_tcp
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_sub(1)
            })
            .ok();
    }

    pub fn record_tcp_open_attempt(&self) {
        self.tcp_open_attempts.fetch_add(1, Ordering::Relaxed);
    }

    pub async fn record_tcp_open_success(&self) {
        self.tcp_open_successes.fetch_add(1, Ordering::Relaxed);
        *self.last_error.lock().await = None;
    }

    pub async fn record_tcp_open_failure(&self, err: impl Into<String>) {
        self.tcp_open_failures.fetch_add(1, Ordering::Relaxed);
        *self.last_error.lock().await = Some(err.into());
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

    async fn record_quic_flow_close(&self, reason: impl Into<String>, reset: bool) {
        if reset {
            self.quic_flow_resets.fetch_add(1, Ordering::Relaxed);
        } else {
            self.quic_flow_graceful_closes
                .fetch_add(1, Ordering::Relaxed);
        }
        *self.last_quic_flow_close_reason.lock().await = Some(reason.into());
    }

    fn record_quic_flow_drop(&self) {
        self.quic_flow_resets.fetch_add(1, Ordering::Relaxed);
    }

    fn record_quic_flow_first_byte_latency(&self, latency: Duration) {
        record_latency_sample(
            &self.quic_flow_first_byte_samples,
            &self.last_quic_flow_first_byte_latency_ms,
            &self.max_quic_flow_first_byte_latency_ms,
            latency,
        );
    }

    fn record_quic_stream_open_latency(&self, latency: Duration) {
        record_latency_sample(
            &self.quic_stream_open_samples,
            &self.last_quic_stream_open_latency_ms,
            &self.max_quic_stream_open_latency_ms,
            latency,
        );
    }

    fn record_quic_header_write_latency(&self, latency: Duration) {
        record_latency_sample(
            &self.quic_header_write_samples,
            &self.last_quic_header_write_latency_ms,
            &self.max_quic_header_write_latency_ms,
            latency,
        );
    }

    pub fn record_quic_copy(
        &self,
        duration: Duration,
        client_to_remote_bytes: u64,
        remote_to_client_bytes: u64,
    ) {
        record_latency_sample(
            &self.quic_copy_duration_samples,
            &self.last_quic_copy_duration_ms,
            &self.max_quic_copy_duration_ms,
            duration,
        );
        self.last_quic_copy_client_to_remote_bytes
            .store(client_to_remote_bytes, Ordering::Relaxed);
        self.last_quic_copy_remote_to_client_bytes
            .store(remote_to_client_bytes, Ordering::Relaxed);
        update_max(
            &self.max_quic_copy_client_to_remote_bytes,
            client_to_remote_bytes,
        );
        update_max(
            &self.max_quic_copy_remote_to_client_bytes,
            remote_to_client_bytes,
        );
    }

    pub fn record_quic_copy_failure(&self) {
        self.quic_copy_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_quic_backpressure_timeout(&self) {
        self.quic_backpressure_timeouts
            .fetch_add(1, Ordering::Relaxed);
    }

    pub async fn status_value(&self) -> serde_json::Value {
        status::status_value(self).await
    }

    async fn status_json(&self) -> Result<String> {
        status::status_json(self).await
    }

    pub fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.shutdown_notify.notify_waiters();
    }

    async fn shutdown_error(&self) -> Option<String> {
        self.last_control_error.lock().await.clone()
    }

    async fn shutdown_notified(&self) {
        if self.shutdown.load(Ordering::Relaxed) {
            return;
        }
        self.shutdown_notify.notified().await;
    }
}

pub async fn run_with_state(args: cli::ProxyArgs, state: Arc<State>) -> Result<()> {
    if let Some(addr) = args.control_listen {
        let control_state = state.clone();
        tokio::spawn(async move {
            if let Err(err) = run_control_server(addr, control_state).await {
                warn!(%addr, error = %err, "quic-native control server stopped");
            }
        });
    }

    let listener = TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("failed to bind quic-native proxy listener {}", args.listen))?;
    info!(listen = %args.listen, "quic-native proxy listening");

    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, peer) = accept?;
                let _ = stream.set_nodelay(true);
                let state = state.clone();
                let tcp_target = args.tcp_target.clone();
                tokio::spawn(async move {
                    let result = if let Some(target) = tcp_target {
                        socks::handle_fixed_target_quic_native(stream, peer, target, state).await
                    } else {
                        socks::handle_client_quic_native(stream, peer, state).await
                    };
                    if let Err(err) = result {
                        debug!(%peer, error = %err, "quic-native proxy client failed");
                    }
                });
            }
            _ = state.shutdown_notified() => {
                info!("shutdown requested; stopping quic-native proxy listener");
                break;
            }
        }
    }
    if let Some(err) = state.shutdown_error().await {
        return Err(anyhow!("quic-native control stream degraded: {err}"));
    }
    Ok(())
}

pub async fn run_with_slot(args: cli::ProxyArgs, slot: Arc<StateSlot>) -> Result<()> {
    let state = State::connect(args.clone()).await?;
    slot.set_current(state.clone()).await;
    let result = run_with_state(args, state.clone()).await;
    let err = result.as_ref().err().map(|err| err.to_string());
    slot.clear_current(&state, err).await;
    result
}

async fn run_control_server(addr: SocketAddr, state: Arc<State>) -> Result<()> {
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind quic-native control listener {addr}"))?;
    info!(%addr, "quic-native control listener ready");
    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, peer) = accept?;
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_control(stream, state).await {
                        warn!(%peer, error = %err, "quic-native control request failed");
                    }
                });
            }
            _ = state.shutdown_notified() => break,
        }
    }
    Ok(())
}

async fn handle_control(stream: TcpStream, state: Arc<State>) -> Result<()> {
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
    use crate::peer_transport;
    use crate::quic_native::pool::QUIC_CONNECTION_POOL_SELECTION_POLICY;

    #[tokio::test]
    async fn state_slot_reports_quic_native_disconnected_status() {
        let slot = StateSlot::new();

        let status = slot.status_value().await;

        assert_eq!(status["connected"], false);
        assert_eq!(status["selected_protocol"], "quic-native");
        assert_eq!(status["quic_mode"], "native-per-flow");
        assert_eq!(status["quic_connection_pool_size"], 0);
        assert_eq!(status["quic_connection_pool_policy"], "disconnected");
        assert_eq!(
            status["quic_connection_pool_workload_hint"],
            serde_json::Value::Null
        );
        assert_eq!(
            status["quic_connection_pool_reason"],
            "no active QUIC-native route"
        );
        assert_eq!(status["quic_connection_pool_mode"], "disconnected");
        assert_eq!(
            status["quic_connection_pool_selection_policy"],
            QUIC_CONNECTION_POOL_SELECTION_POLICY
        );
        assert_eq!(status["active_quic_connections"], 0);
        assert_eq!(
            status["quic_connections"].as_array().expect("connections"),
            &Vec::<serde_json::Value>::new()
        );
        assert_eq!(status["active_quic_flows"], 0);
        assert_eq!(status["quic_stream_open_samples"], 0);
        assert_eq!(
            status["last_quic_stream_open_latency_ms"],
            serde_json::Value::Null
        );
        assert_eq!(
            status["max_quic_stream_open_latency_ms"],
            serde_json::Value::Null
        );
        assert_eq!(status["quic_stream_open_failures"], 0);
        assert_eq!(status["quic_header_write_samples"], 0);
        assert_eq!(
            status["last_quic_header_write_latency_ms"],
            serde_json::Value::Null
        );
        assert_eq!(
            status["max_quic_header_write_latency_ms"],
            serde_json::Value::Null
        );
        assert_eq!(status["quic_header_write_failures"], 0);
        assert_eq!(status["quic_backpressure_timeouts"], 0);
        assert_eq!(status["quic_flow_graceful_closes"], 0);
        assert_eq!(status["quic_flow_resets"], 0);
        assert_eq!(status["quic_flow_first_byte_samples"], 0);
        assert_eq!(status["control_state"], "disconnected");
        assert_eq!(status["control_degraded"], true);
        assert_eq!(status["control_pings_sent"], 0);
        assert_eq!(status["control_pongs_received"], 0);
        assert_eq!(
            status["last_control_pong_latency_ms"],
            serde_json::Value::Null
        );
        assert_eq!(
            status["control_keepalive_interval_secs"],
            CONTROL_KEEPALIVE_INTERVAL.as_secs()
        );
        assert_eq!(
            status["control_keepalive_timeout_secs"],
            CONTROL_KEEPALIVE_TIMEOUT.as_secs()
        );
        assert_eq!(
            status["last_quic_flow_first_byte_latency_ms"],
            serde_json::Value::Null
        );
        assert_eq!(
            status["max_quic_flow_first_byte_latency_ms"],
            serde_json::Value::Null
        );
        assert_eq!(status["quic_copy_duration_samples"], 0);
        assert_eq!(
            status["last_quic_copy_duration_ms"],
            serde_json::Value::Null
        );
        assert_eq!(status["max_quic_copy_duration_ms"], serde_json::Value::Null);
        assert_eq!(status["quic_copy_failures"], 0);
        assert_eq!(
            status["quic_copy_buffer_size"],
            QUIC_NATIVE_COPY_BUFFER_SIZE
        );
        assert_eq!(
            status["quic_stream_open_timeout_secs"],
            serde_json::Value::Null
        );
        assert_eq!(
            status["quic_first_byte_timeout_secs"],
            QUIC_NATIVE_FIRST_BYTE_TIMEOUT.as_secs()
        );
        assert_eq!(
            status["quic_backpressure_timeout_secs"],
            QUIC_NATIVE_BACKPRESSURE_TIMEOUT.as_secs()
        );
        assert_eq!(
            status["last_quic_copy_client_to_remote_bytes"],
            serde_json::Value::Null
        );
        assert_eq!(
            status["last_quic_copy_remote_to_client_bytes"],
            serde_json::Value::Null
        );
        assert_eq!(
            status["quic_max_bidi_streams"],
            peer_transport::QUIC_MAX_BIDI_STREAMS
        );
        assert_eq!(status["quic_udp_runtime"], peer_transport::QUIC_UDP_RUNTIME);
        assert_eq!(status["quic_udp_gso"], serde_json::Value::Null);
        assert_eq!(
            status["quic_packetization"],
            peer_transport::QUIC_PACKETIZATION
        );
        assert_eq!(status["quic_profile"]["connected"], false);
        assert_eq!(status["quic_profile"]["mode"], "native-per-flow");
        assert_eq!(status["quic_profile"]["pool"]["size"], 0);
        assert_eq!(
            status["quic_profile"]["pool"]["selection_policy"],
            QUIC_CONNECTION_POOL_SELECTION_POLICY
        );
        assert_eq!(
            status["quic_profile"]["signals"]["next_bottleneck"],
            "disconnected"
        );
        assert_eq!(status["quic_profile"]["flow"]["stream_open_samples"], 0);
        assert_eq!(
            status["quic_runtime"]["transport_options"]["max_bidi_streams"],
            peer_transport::QUIC_MAX_BIDI_STREAMS
        );
        assert_eq!(status["link"]["health"]["selected_protocol"], "quic-native");
        assert_eq!(status["link"]["health"]["active_connections"], 0);
        assert_eq!(status["link"]["health"]["active_streams"], 0);
        assert_eq!(status["link"]["health"]["active_channels"], 0);
        assert_eq!(status["link"]["health"]["pool_size"], 0);
        assert_eq!(
            status["link"]["health"]["pool_selection_policy"],
            QUIC_CONNECTION_POOL_SELECTION_POLICY
        );
        assert_eq!(status["link"]["health"]["open_attempts"], 0);
        assert_eq!(status["link"]["health"]["open_successes"], 0);
        assert_eq!(status["link"]["health"]["open_failures"], 0);
        assert_eq!(
            status["link"]["health"]["open_latency_ms"],
            serde_json::Value::Null
        );
        assert_eq!(status["link"]["health"]["first_byte_samples"], 0);
        assert_eq!(
            status["link"]["health"]["first_byte_latency_ms"],
            serde_json::Value::Null
        );
        assert_eq!(
            status["link"]["health"]["max_first_byte_latency_ms"],
            serde_json::Value::Null
        );
        assert_eq!(
            status["link"]["health"]["last_close_reason"],
            serde_json::Value::Null
        );
        assert_eq!(
            status["link"]["health"]["degraded_reason"],
            serde_json::Value::Null
        );
        assert_eq!(status["link"]["health"]["control_health"], "disconnected");
    }
}
