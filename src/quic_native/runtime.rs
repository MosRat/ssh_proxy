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

use super::metrics::{duration_millis, last_sampled_u64, record_latency_sample, update_max};
use super::pool::{
    QUIC_CONNECTION_POOL_SELECTION_POLICY, quic_connection_pool_mode, quic_connection_pool_policy,
    quic_connection_pool_reason, quic_connection_pool_workload_hint, quic_profile_next_bottleneck,
    quic_worker_score,
};
use super::runtime_config::{local_node_name, quic_options_from_proxy_args, route_id};
use super::runtime_status;

mod control_loop;
mod stream;

use control_loop::run_control_loop;

const CONTROL_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const CONTROL_KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(45);
pub const QUIC_NATIVE_COPY_BUFFER_SIZE: usize = crate::protocol::TCP_DATA_CHUNK;
pub const QUIC_NATIVE_BACKPRESSURE_TIMEOUT: Duration = Duration::from_secs(120);
pub const QUIC_NATIVE_FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(15);

use crate::{
    cli, peer_transport,
    quic_native::{
        flow::write_flow_header_with_buffer,
        session::{RouteSessionSpec, client_negotiate},
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

struct ConnectedWorker {
    worker: Arc<ConnectionWorker>,
    control: quic_stream::QuicBiStream,
    route_id: String,
    _session_node: String,
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

async fn connect_worker(
    args: &cli::ProxyArgs,
    route_id: &str,
    worker_id: usize,
) -> Result<ConnectedWorker> {
    let addr = args
        .remote_quic
        .ok_or_else(|| anyhow!("--remote-quic is required for quic-native transport"))?;
    let ca = args
        .remote_ca
        .as_deref()
        .ok_or_else(|| anyhow!("--remote-ca is required for quic-native transport"))?;
    let roots = peer_transport::load_cert_chain(ca)?;
    let quic_options = quic_options_from_proxy_args(args)?;
    debug!(
        worker_id,
        remote_quic = %addr,
        remote_name = %args.remote_name,
        quic_udp_runtime = peer_transport::QUIC_UDP_RUNTIME,
        quic_udp_gso_source = peer_transport::QUIC_UDP_GSO_SOURCE,
        quic_packetization = peer_transport::QUIC_PACKETIZATION,
        ?quic_options,
        "connecting QUIC-native worker"
    );
    let mut endpoint = quinn::Endpoint::client(SocketAddr::from(([0, 0, 0, 0], 0)))
        .context("failed to create QUIC-native client endpoint")?;
    endpoint.set_default_client_config(peer_transport::quic_client_config(roots, quic_options)?);
    let connecting = endpoint
        .connect(addr, &args.remote_name)
        .context("failed to create QUIC-native connect request")?;
    let connection = time::timeout(
        Duration::from_secs(args.connect_timeout_secs.max(1)),
        connecting,
    )
    .await
    .with_context(|| {
        format!(
            "remote QUIC-native transport {addr} timed out after {}s",
            args.connect_timeout_secs.max(1)
        )
    })?
    .with_context(|| format!("failed to connect remote QUIC-native transport {addr}"))?;
    let control_open_started = Instant::now();
    let (send, recv) = time::timeout(
        Duration::from_secs(args.connect_timeout_secs.max(1)),
        connection.open_bi(),
    )
    .await
    .with_context(|| {
        format!(
            "remote QUIC-native control stream open timed out after {}s",
            args.connect_timeout_secs.max(1)
        )
    })?
    .context("failed to open QUIC-native control stream")?;
    debug!(
        worker_id,
        control_open_latency_ms = duration_millis(control_open_started.elapsed()),
        "opened QUIC-native control stream"
    );
    let mut control =
        quic_stream::QuicBiStream::with_lifetime(send, recv, connection.clone(), endpoint);
    peer_transport::client_handshake(
        &mut control,
        local_node_name(),
        peer_transport::PeerProtocol::QuicNative,
    )
    .await?;
    let session = client_negotiate(
        &mut control,
        RouteSessionSpec::new(
            route_id.to_string(),
            local_node_name(),
            peer_transport::default_features(),
            vec![peer_transport::PeerProtocol::QuicNative.to_string()],
        ),
    )
    .await?;
    let worker = Arc::new(ConnectionWorker::new(worker_id, connection));
    Ok(ConnectedWorker {
        worker,
        control,
        route_id: session.welcome.route_id,
        _session_node: session.hello.node,
    })
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

    async fn connection_status_values(&self) -> Vec<serde_json::Value> {
        let mut values = Vec::with_capacity(self.workers.len());
        for worker in &self.workers {
            values.push(serde_json::json!({
                "worker_id": worker.id,
                "connected": !worker.control_degraded.load(Ordering::Relaxed),
                "uptime_secs": worker.started.elapsed().as_secs(),
                "active_quic_flows": worker.active_quic_flows.load(Ordering::Relaxed),
                "opened_quic_flows": worker.opened_quic_flows.load(Ordering::Relaxed),
                "stream_open_failures": worker.stream_open_failures.load(Ordering::Relaxed),
                "last_stream_open_latency_ms": last_sampled_u64(
                    worker.opened_quic_flows.load(Ordering::Relaxed),
                    worker.last_stream_open_latency_ms.load(Ordering::Relaxed),
                ),
                "bytes_client_to_remote": worker.bytes_client_to_remote.load(Ordering::Relaxed),
                "bytes_remote_to_client": worker.bytes_remote_to_client.load(Ordering::Relaxed),
                "quic_flow_graceful_closes": worker.quic_flow_graceful_closes.load(Ordering::Relaxed),
                "quic_flow_resets": worker.quic_flow_resets.load(Ordering::Relaxed),
                "quic_backpressure_timeouts": worker.quic_backpressure_timeouts.load(Ordering::Relaxed),
                "control_state": if worker.control_degraded.load(Ordering::Relaxed) {
                    "degraded"
                } else {
                    "healthy"
                },
                "control_degraded": worker.control_degraded.load(Ordering::Relaxed),
                "control_pings_sent": worker.control_pings_sent.load(Ordering::Relaxed),
                "control_pongs_received": worker.control_pongs_received.load(Ordering::Relaxed),
                "last_control_pong_latency_ms": last_sampled_u64(
                    worker.control_pongs_received.load(Ordering::Relaxed),
                    worker.last_control_pong_latency_ms.load(Ordering::Relaxed),
                ),
                "score_components": {
                    "active_quic_flows": worker.active_quic_flows.load(Ordering::Relaxed),
                    "stream_open_failures": worker.stream_open_failures.load(Ordering::Relaxed),
                    "last_stream_open_latency_ms": worker.last_stream_open_latency_ms.load(Ordering::Relaxed),
                    "quic_backpressure_timeouts": worker.quic_backpressure_timeouts.load(Ordering::Relaxed),
                    "quic_flow_resets": worker.quic_flow_resets.load(Ordering::Relaxed),
                    "control_degraded": worker.control_degraded.load(Ordering::Relaxed),
                },
                "last_control_error": worker.last_control_error.lock().await.clone(),
                "selection_score": worker.score(),
            }));
        }
        values
    }

    pub async fn status_value(&self) -> serde_json::Value {
        let quic_connections = self.connection_status_values().await;
        let quic_options = quic_options_from_proxy_args(&self.args).unwrap_or_default();
        let quic_runtime = peer_transport::quic_runtime_diagnostics(quic_options);
        let active_quic_connections = self.workers.len();
        let active_quic_flows = self.active_quic_flows.load(Ordering::Relaxed);
        let quic_stream_open_samples = self.quic_stream_open_samples.load(Ordering::Relaxed);
        let quic_stream_open_failures = self.quic_stream_open_failures.load(Ordering::Relaxed);
        let quic_header_write_samples = self.quic_header_write_samples.load(Ordering::Relaxed);
        let quic_header_write_failures = self.quic_header_write_failures.load(Ordering::Relaxed);
        let quic_backpressure_timeouts = self.quic_backpressure_timeouts.load(Ordering::Relaxed);
        let quic_flow_graceful_closes = self.quic_flow_graceful_closes.load(Ordering::Relaxed);
        let quic_flow_resets = self.quic_flow_resets.load(Ordering::Relaxed);
        let first_byte_samples = self.quic_flow_first_byte_samples.load(Ordering::Relaxed);
        let last_quic_flow_first_byte_latency_ms = last_sampled_u64(
            first_byte_samples,
            self.last_quic_flow_first_byte_latency_ms
                .load(Ordering::Relaxed),
        );
        let max_quic_flow_first_byte_latency_ms = last_sampled_u64(
            first_byte_samples,
            self.max_quic_flow_first_byte_latency_ms
                .load(Ordering::Relaxed),
        );
        let quic_copy_duration_samples = self.quic_copy_duration_samples.load(Ordering::Relaxed);
        let last_quic_copy_duration_ms = last_sampled_u64(
            quic_copy_duration_samples,
            self.last_quic_copy_duration_ms.load(Ordering::Relaxed),
        );
        let max_quic_copy_duration_ms = last_sampled_u64(
            quic_copy_duration_samples,
            self.max_quic_copy_duration_ms.load(Ordering::Relaxed),
        );
        let quic_copy_failures = self.quic_copy_failures.load(Ordering::Relaxed);
        let last_quic_copy_client_to_remote_bytes = last_sampled_u64(
            quic_copy_duration_samples,
            self.last_quic_copy_client_to_remote_bytes
                .load(Ordering::Relaxed),
        );
        let last_quic_copy_remote_to_client_bytes = last_sampled_u64(
            quic_copy_duration_samples,
            self.last_quic_copy_remote_to_client_bytes
                .load(Ordering::Relaxed),
        );
        let max_quic_copy_client_to_remote_bytes = last_sampled_u64(
            quic_copy_duration_samples,
            self.max_quic_copy_client_to_remote_bytes
                .load(Ordering::Relaxed),
        );
        let max_quic_copy_remote_to_client_bytes = last_sampled_u64(
            quic_copy_duration_samples,
            self.max_quic_copy_remote_to_client_bytes
                .load(Ordering::Relaxed),
        );
        let bytes_client_to_remote = self.bytes_client_to_remote.load(Ordering::Relaxed);
        let bytes_remote_to_client = self.bytes_remote_to_client.load(Ordering::Relaxed);
        let control_degraded = self.control_degraded.load(Ordering::Relaxed);
        let control_state = if control_degraded {
            "degraded"
        } else {
            "healthy"
        };
        let control_pings_sent = self.control_pings_sent.load(Ordering::Relaxed);
        let control_pongs_received = self.control_pongs_received.load(Ordering::Relaxed);
        let last_control_pong_latency_ms = last_sampled_u64(
            control_pongs_received,
            self.last_control_pong_latency_ms.load(Ordering::Relaxed),
        );
        let last_control_error = self.last_control_error.lock().await.clone();
        let last_quic_flow_close_reason = self.last_quic_flow_close_reason.lock().await.clone();
        let last_error = self.last_error.lock().await.clone();
        let mut status = serde_json::Map::new();
        status.insert(
            "connected".to_string(),
            (!self.shutdown.load(Ordering::Relaxed)).into(),
        );
        status.insert("selected_protocol".to_string(), "quic-native".into());
        status.insert("quic_mode".to_string(), "native-per-flow".into());
        status.insert("route_id".to_string(), self.route_id.clone().into());
        status.insert(
            "remote_quic".to_string(),
            serde_json::to_value(self.args.remote_quic).expect("remote_quic serializable"),
        );
        status.insert(
            "remote_name".to_string(),
            self.args.remote_name.clone().into(),
        );
        status.insert(
            "egress_proxy".to_string(),
            serde_json::to_value(self.args.egress_proxy.clone())
                .expect("egress proxy serializable"),
        );
        status.insert(
            "uptime_secs".to_string(),
            self.started.elapsed().as_secs().into(),
        );
        status.insert(
            "quic_connection_pool_size".to_string(),
            self.args.transport_pool_size.max(1).into(),
        );
        status.insert(
            "quic_connection_pool_policy".to_string(),
            quic_connection_pool_policy(&self.args).into(),
        );
        status.insert(
            "quic_connection_pool_workload_hint".to_string(),
            serde_json::to_value(quic_connection_pool_workload_hint(&self.args))
                .expect("quic pool workload hint serializable"),
        );
        status.insert(
            "quic_connection_pool_reason".to_string(),
            quic_connection_pool_reason(&self.args).into(),
        );
        status.insert(
            "quic_connection_pool_mode".to_string(),
            quic_connection_pool_mode(self.args.transport_pool_size.max(1)).into(),
        );
        status.insert(
            "quic_connection_pool_selection_policy".to_string(),
            QUIC_CONNECTION_POOL_SELECTION_POLICY.into(),
        );
        status.insert(
            "active_quic_connections".to_string(),
            active_quic_connections.into(),
        );
        status.insert(
            "quic_connections".to_string(),
            serde_json::Value::Array(quic_connections.clone()),
        );
        status.insert("active_quic_flows".to_string(), active_quic_flows.into());
        status.insert(
            "quic_stream_open_samples".to_string(),
            quic_stream_open_samples.into(),
        );
        status.insert(
            "last_quic_stream_open_latency_ms".to_string(),
            serde_json::to_value(last_sampled_u64(
                quic_stream_open_samples,
                self.last_quic_stream_open_latency_ms
                    .load(Ordering::Relaxed),
            ))
            .expect("stream open latency serializable"),
        );
        status.insert(
            "max_quic_stream_open_latency_ms".to_string(),
            serde_json::to_value(last_sampled_u64(
                quic_stream_open_samples,
                self.max_quic_stream_open_latency_ms.load(Ordering::Relaxed),
            ))
            .expect("max stream open latency serializable"),
        );
        status.insert(
            "quic_stream_open_failures".to_string(),
            quic_stream_open_failures.into(),
        );
        status.insert(
            "quic_header_write_samples".to_string(),
            self.quic_header_write_samples
                .load(Ordering::Relaxed)
                .into(),
        );
        status.insert(
            "last_quic_header_write_latency_ms".to_string(),
            serde_json::to_value(last_sampled_u64(
                self.quic_header_write_samples.load(Ordering::Relaxed),
                self.last_quic_header_write_latency_ms
                    .load(Ordering::Relaxed),
            ))
            .expect("header write latency serializable"),
        );
        status.insert(
            "max_quic_header_write_latency_ms".to_string(),
            serde_json::to_value(last_sampled_u64(
                self.quic_header_write_samples.load(Ordering::Relaxed),
                self.max_quic_header_write_latency_ms
                    .load(Ordering::Relaxed),
            ))
            .expect("max header write latency serializable"),
        );
        status.insert(
            "quic_header_write_failures".to_string(),
            self.quic_header_write_failures
                .load(Ordering::Relaxed)
                .into(),
        );
        status.insert(
            "quic_backpressure_timeouts".to_string(),
            self.quic_backpressure_timeouts
                .load(Ordering::Relaxed)
                .into(),
        );
        status.insert(
            "quic_flow_graceful_closes".to_string(),
            self.quic_flow_graceful_closes
                .load(Ordering::Relaxed)
                .into(),
        );
        status.insert(
            "quic_flow_resets".to_string(),
            self.quic_flow_resets.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "quic_flow_first_byte_samples".to_string(),
            first_byte_samples.into(),
        );
        status.insert(
            "last_quic_flow_first_byte_latency_ms".to_string(),
            serde_json::to_value(last_quic_flow_first_byte_latency_ms)
                .expect("first byte latency serializable"),
        );
        status.insert(
            "max_quic_flow_first_byte_latency_ms".to_string(),
            serde_json::to_value(last_sampled_u64(
                first_byte_samples,
                self.max_quic_flow_first_byte_latency_ms
                    .load(Ordering::Relaxed),
            ))
            .expect("max first byte latency serializable"),
        );
        status.insert(
            "quic_copy_duration_samples".to_string(),
            self.quic_copy_duration_samples
                .load(Ordering::Relaxed)
                .into(),
        );
        status.insert(
            "last_quic_copy_duration_ms".to_string(),
            serde_json::to_value(last_sampled_u64(
                self.quic_copy_duration_samples.load(Ordering::Relaxed),
                self.last_quic_copy_duration_ms.load(Ordering::Relaxed),
            ))
            .expect("copy duration serializable"),
        );
        status.insert(
            "max_quic_copy_duration_ms".to_string(),
            serde_json::to_value(last_sampled_u64(
                self.quic_copy_duration_samples.load(Ordering::Relaxed),
                self.max_quic_copy_duration_ms.load(Ordering::Relaxed),
            ))
            .expect("max copy duration serializable"),
        );
        status.insert(
            "quic_copy_failures".to_string(),
            self.quic_copy_failures.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "last_quic_copy_client_to_remote_bytes".to_string(),
            serde_json::to_value(last_sampled_u64(
                self.quic_copy_duration_samples.load(Ordering::Relaxed),
                self.last_quic_copy_client_to_remote_bytes
                    .load(Ordering::Relaxed),
            ))
            .expect("copy bytes serializable"),
        );
        status.insert(
            "last_quic_copy_remote_to_client_bytes".to_string(),
            serde_json::to_value(last_sampled_u64(
                self.quic_copy_duration_samples.load(Ordering::Relaxed),
                self.last_quic_copy_remote_to_client_bytes
                    .load(Ordering::Relaxed),
            ))
            .expect("copy bytes serializable"),
        );
        status.insert(
            "max_quic_copy_client_to_remote_bytes".to_string(),
            serde_json::to_value(last_sampled_u64(
                self.quic_copy_duration_samples.load(Ordering::Relaxed),
                self.max_quic_copy_client_to_remote_bytes
                    .load(Ordering::Relaxed),
            ))
            .expect("max copy bytes serializable"),
        );
        status.insert(
            "max_quic_copy_remote_to_client_bytes".to_string(),
            serde_json::to_value(last_sampled_u64(
                self.quic_copy_duration_samples.load(Ordering::Relaxed),
                self.max_quic_copy_remote_to_client_bytes
                    .load(Ordering::Relaxed),
            ))
            .expect("max copy bytes serializable"),
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
            "bytes_client_to_remote".to_string(),
            bytes_client_to_remote.into(),
        );
        status.insert(
            "bytes_remote_to_client".to_string(),
            bytes_remote_to_client.into(),
        );
        status.insert("control_state".to_string(), control_state.into());
        status.insert("control_degraded".to_string(), control_degraded.into());
        status.insert(
            "control_pings_sent".to_string(),
            self.control_pings_sent.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "control_pongs_received".to_string(),
            self.control_pongs_received.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "last_control_pong_latency_ms".to_string(),
            serde_json::to_value(last_sampled_u64(
                self.control_pongs_received.load(Ordering::Relaxed),
                self.last_control_pong_latency_ms.load(Ordering::Relaxed),
            ))
            .expect("control pong latency serializable"),
        );
        status.insert(
            "control_keepalive_interval_secs".to_string(),
            CONTROL_KEEPALIVE_INTERVAL.as_secs().into(),
        );
        status.insert(
            "control_keepalive_timeout_secs".to_string(),
            CONTROL_KEEPALIVE_TIMEOUT.as_secs().into(),
        );
        status.insert(
            "last_control_error".to_string(),
            serde_json::to_value(last_control_error.clone()).expect("control error serializable"),
        );
        status.insert(
            "read_buffer_size".to_string(),
            QUIC_NATIVE_COPY_BUFFER_SIZE.into(),
        );
        status.insert(
            "quic_copy_buffer_size".to_string(),
            QUIC_NATIVE_COPY_BUFFER_SIZE.into(),
        );
        status.insert(
            "quic_stream_open_timeout_secs".to_string(),
            self.args.connect_timeout_secs.max(1).into(),
        );
        status.insert(
            "quic_first_byte_timeout_secs".to_string(),
            QUIC_NATIVE_FIRST_BYTE_TIMEOUT.as_secs().into(),
        );
        status.insert(
            "quic_backpressure_timeout_secs".to_string(),
            QUIC_NATIVE_BACKPRESSURE_TIMEOUT.as_secs().into(),
        );
        status.insert(
            "write_batch_limit".to_string(),
            crate::protocol::FRAME_WRITE_BATCH_LIMIT.into(),
        );
        status.insert(
            "frame_channel_capacity".to_string(),
            crate::protocol::FRAME_CHANNEL_CAPACITY.into(),
        );
        status.insert(
            "quic_receive_window".to_string(),
            quic_options.receive_window.into(),
        );
        status.insert(
            "quic_stream_receive_window".to_string(),
            quic_options.stream_receive_window.into(),
        );
        status.insert(
            "quic_max_bidi_streams".to_string(),
            quic_options.max_bidi_streams.into(),
        );
        status.insert(
            "quic_keep_alive_interval_secs".to_string(),
            quic_options.keep_alive_interval_secs.into(),
        );
        status.insert(
            "quic_idle_timeout_secs".to_string(),
            quic_options.idle_timeout_secs.into(),
        );
        status.insert(
            "quic_runtime".to_string(),
            serde_json::to_value(quic_runtime.clone())
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
            "quic_profile".to_string(),
            serde_json::json!({
                "enabled": true,
                "profile_scope": "quic-native-route-status",
                "connected": !self.shutdown.load(Ordering::Relaxed),
                "mode": "native-per-flow",
                "route_id": self.route_id.clone(),
                "active_connections": active_quic_connections,
                "active_flows": active_quic_flows,
                "pool": {
                    "size": self.args.transport_pool_size.max(1),
                    "active_connections": active_quic_connections,
                    "policy": quic_connection_pool_policy(&self.args),
                    "workload_hint": quic_connection_pool_workload_hint(&self.args),
                    "reason": quic_connection_pool_reason(&self.args),
                    "mode": quic_connection_pool_mode(self.args.transport_pool_size.max(1)),
                    "selection_policy": QUIC_CONNECTION_POOL_SELECTION_POLICY,
                },
                "transport": quic_runtime.transport_options,
                "runtime": quic_runtime.clone(),
                "udp": {
                    "runtime": peer_transport::QUIC_UDP_RUNTIME,
                    "gso": serde_json::Value::Null,
                    "gso_source": peer_transport::QUIC_UDP_GSO_SOURCE,
                    "packetization": peer_transport::QUIC_PACKETIZATION,
                    "max_datagram_size": serde_json::Value::Null,
                    "max_datagram_size_source": "unavailable: Quinn endpoint API is not exposed through ssh_proxy status",
                    "packet_loss": serde_json::Value::Null,
                    "packet_loss_source": "unavailable: Quinn connection loss counters are not exposed through ssh_proxy status",
                    "congestion_signal": serde_json::Value::Null,
                    "congestion_signal_source": "unavailable: Quinn congestion counters are not exposed through ssh_proxy status",
                },
                "connections": quic_connections.clone(),
                "flow": {
                    "stream_open_samples": quic_stream_open_samples,
                    "stream_open_failures": quic_stream_open_failures,
                    "last_stream_open_latency_ms": serde_json::to_value(last_sampled_u64(
                        quic_stream_open_samples,
                        self.last_quic_stream_open_latency_ms.load(Ordering::Relaxed),
                    )).expect("stream open latency serializable"),
                    "max_stream_open_latency_ms": serde_json::to_value(last_sampled_u64(
                        quic_stream_open_samples,
                        self.max_quic_stream_open_latency_ms.load(Ordering::Relaxed),
                    )).expect("max stream open latency serializable"),
                    "header_write_samples": quic_header_write_samples,
                    "header_write_failures": quic_header_write_failures,
                    "last_header_write_latency_ms": serde_json::to_value(last_sampled_u64(
                        quic_header_write_samples,
                        self.last_quic_header_write_latency_ms.load(Ordering::Relaxed),
                    )).expect("header write latency serializable"),
                    "max_header_write_latency_ms": serde_json::to_value(last_sampled_u64(
                        quic_header_write_samples,
                        self.max_quic_header_write_latency_ms.load(Ordering::Relaxed),
                    )).expect("max header write latency serializable"),
                    "first_byte_samples": first_byte_samples,
                    "last_first_byte_latency_ms": serde_json::to_value(last_quic_flow_first_byte_latency_ms).expect("first byte latency serializable"),
                    "max_first_byte_latency_ms": serde_json::to_value(max_quic_flow_first_byte_latency_ms).expect("max first byte latency serializable"),
                    "copy_duration_samples": quic_copy_duration_samples,
                    "last_copy_duration_ms": serde_json::to_value(last_quic_copy_duration_ms).expect("copy duration serializable"),
                    "max_copy_duration_ms": serde_json::to_value(max_quic_copy_duration_ms).expect("max copy duration serializable"),
                    "copy_failures": quic_copy_failures,
                    "last_copy_client_to_remote_bytes": serde_json::to_value(last_quic_copy_client_to_remote_bytes).expect("copy bytes serializable"),
                    "last_copy_remote_to_client_bytes": serde_json::to_value(last_quic_copy_remote_to_client_bytes).expect("copy bytes serializable"),
                    "max_copy_client_to_remote_bytes": serde_json::to_value(max_quic_copy_client_to_remote_bytes).expect("max copy bytes serializable"),
                    "max_copy_remote_to_client_bytes": serde_json::to_value(max_quic_copy_remote_to_client_bytes).expect("max copy bytes serializable"),
                    "backpressure_timeouts": quic_backpressure_timeouts,
                    "graceful_closes": quic_flow_graceful_closes,
                    "resets": quic_flow_resets,
                },
                "control": {
                    "state": control_state,
                    "degraded": control_degraded,
                    "pings_sent": control_pings_sent,
                    "pongs_received": control_pongs_received,
                    "last_pong_latency_ms": serde_json::to_value(last_control_pong_latency_ms).expect("control pong latency serializable"),
                    "last_error": last_control_error.clone(),
                },
                "signals": {
                    "next_bottleneck": quic_profile_next_bottleneck(
                        control_degraded,
                        quic_backpressure_timeouts,
                        quic_copy_failures,
                        quic_copy_duration_samples,
                        quic_stream_open_failures,
                        quic_header_write_failures,
                    ),
                    "window_sizing": {
                        "suspected": quic_stream_open_failures > 0 || quic_header_write_failures > 0,
                        "evidence": if quic_stream_open_failures > 0 || quic_header_write_failures > 0 {
                            "stream or header opens are failing"
                        } else if quic_stream_open_samples == 0 {
                            "no successful QUIC stream opens recorded"
                        } else {
                            "stream opens are currently healthy"
                        },
                    },
                    "udp_path": {
                        "suspected": quic_runtime.udp_gso.is_none(),
                        "evidence": quic_runtime.udp_gso_source,
                    },
                    "application_copy": {
                        "suspected": quic_copy_failures > 0 || quic_copy_duration_samples > 0,
                        "evidence": if quic_copy_failures > 0 {
                            "copy failures were recorded"
                        } else if quic_copy_duration_samples > 0 {
                            "copy duration samples are present"
                        } else {
                            "no copy samples recorded"
                        },
                    },
                    "slow_consumers": {
                        "suspected": quic_backpressure_timeouts > 0,
                        "evidence": if quic_backpressure_timeouts > 0 {
                            "backpressure timeouts were recorded"
                        } else {
                            "no backpressure timeouts recorded"
                        },
                    },
                    "congestion": {
                        "suspected": false,
                        "evidence": "quinn congestion counters are not exposed through the current status surface",
                    },
                },
            }),
        );
        status.insert(
            "last_quic_flow_close_reason".to_string(),
            serde_json::to_value(last_quic_flow_close_reason.clone())
                .expect("flow close reason serializable"),
        );
        status.insert(
            "last_error".to_string(),
            serde_json::to_value(last_error.clone()).expect("last error serializable"),
        );
        status.insert(
            "link".to_string(),
            serde_json::json!({
                "health": {
                    "selected_protocol": "quic-native",
                    "active_connections": active_quic_connections,
                    "active_streams": active_quic_flows,
                    "active_channels": active_quic_flows,
                    "pool_size": self.args.transport_pool_size.max(1),
                    "pool_policy": quic_connection_pool_policy(&self.args),
                    "pool_workload_hint": quic_connection_pool_workload_hint(&self.args),
                    "pool_reason": quic_connection_pool_reason(&self.args),
                    "pool_mode": quic_connection_pool_mode(self.args.transport_pool_size.max(1)),
                    "pool_selection_policy": QUIC_CONNECTION_POOL_SELECTION_POLICY,
                    "open_attempts": quic_stream_open_samples + quic_stream_open_failures,
                    "open_successes": quic_stream_open_samples,
                    "open_failures": quic_stream_open_failures,
                    "open_latency_ms": last_sampled_u64(
                        quic_stream_open_samples,
                        self.last_quic_stream_open_latency_ms.load(Ordering::Relaxed),
                    ),
                    "bytes_client_to_remote": bytes_client_to_remote,
                    "bytes_remote_to_client": bytes_remote_to_client,
                    "first_byte_samples": first_byte_samples,
                    "first_byte_latency_ms": last_quic_flow_first_byte_latency_ms,
                    "max_first_byte_latency_ms": max_quic_flow_first_byte_latency_ms,
                    "last_close_reason": last_quic_flow_close_reason,
                    "degraded_reason": last_control_error.clone().or(last_error),
                    "control_health": control_state,
                    "connected": !self.shutdown.load(Ordering::Relaxed),
                }
            }),
        );
        serde_json::Value::Object(status)
    }

    async fn status_json(&self) -> Result<String> {
        Ok(format!(
            "{}\n",
            serde_json::to_string_pretty(&self.status_value().await)?
        ))
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
