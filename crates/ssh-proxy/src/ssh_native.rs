use std::{
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    task::{Context as TaskContext, Poll},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use russh::client;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{Mutex, Notify},
    time,
};
use tracing::{debug, info, warn};

use crate::{
    cli, protocol, socks,
    ssh_client::{self, Client},
};

pub struct State {
    args: cli::ProxyArgs,
    sessions: Mutex<Vec<Arc<Session>>>,
    pool_size: usize,
    started: Instant,
    shutdown: AtomicBool,
    shutdown_notify: Notify,
    next_session_id: AtomicU32,
    tcp_open_attempts: AtomicU64,
    tcp_open_successes: AtomicU64,
    tcp_open_failures: AtomicU64,
    last_tcp_open_latency_ms: AtomicU64,
    ssh_session_connect_attempts: AtomicU64,
    ssh_session_connect_failures: AtomicU64,
    ssh_channel_open_attempts: AtomicU64,
    ssh_channel_open_failures: AtomicU64,
    active_ssh_channels: AtomicU64,
    total_tcp: AtomicU64,
    active_tcp: AtomicU64,
    bytes_client_to_remote: AtomicU64,
    bytes_remote_to_client: AtomicU64,
    first_byte_samples: AtomicU64,
    first_byte_latency_total_ms: AtomicU64,
    last_first_byte_latency_ms: AtomicU64,
    max_first_byte_latency_ms: AtomicU64,
    first_byte_latency_buckets: [AtomicU64; FIRST_BYTE_LATENCY_BUCKET_COUNT],
    graceful_closes: AtomicU64,
    error_closes: AtomicU64,
    last_close_reason: StdMutex<Option<String>>,
    session_growth_events: AtomicU64,
    session_growth_suppressed: AtomicU64,
    last_channel_queue_depth: AtomicU64,
    max_channel_queue_depth: AtomicU64,
    last_error: Mutex<Option<String>>,
}

struct Session {
    id: u32,
    client: Client,
    active_channels: AtomicU32,
    opened_channels: AtomicU64,
    open_failures: AtomicU64,
    last_open_latency_ms: AtomicU64,
    last_failure_elapsed_ms: AtomicU64,
    bytes_client_to_remote: AtomicU64,
    bytes_remote_to_client: AtomicU64,
    first_byte_samples: AtomicU64,
    first_byte_latency_total_ms: AtomicU64,
    last_first_byte_latency_ms: AtomicU64,
    max_first_byte_latency_ms: AtomicU64,
    first_byte_latency_buckets: [AtomicU64; FIRST_BYTE_LATENCY_BUCKET_COUNT],
    graceful_closes: AtomicU64,
    error_closes: AtomicU64,
    last_close_reason: StdMutex<Option<String>>,
    last_channel_queue_depth: AtomicU64,
    max_channel_queue_depth: AtomicU64,
}

pub struct Stream {
    inner: russh::ChannelStream<client::Msg>,
    state: Arc<State>,
    session: Arc<Session>,
    opened_at: Instant,
    first_byte_recorded: bool,
    close_recorded: bool,
}

impl State {
    pub fn new(args: cli::ProxyArgs) -> Arc<Self> {
        let pool_size = args
            .ssh_session_pool_size
            .unwrap_or_else(|| if args.tcp_target.is_some() { 1 } else { 2 })
            .max(1);
        Arc::new(Self {
            pool_size,
            args,
            sessions: Mutex::new(Vec::new()),
            started: Instant::now(),
            shutdown: AtomicBool::new(false),
            shutdown_notify: Notify::new(),
            next_session_id: AtomicU32::new(1),
            tcp_open_attempts: AtomicU64::new(0),
            tcp_open_successes: AtomicU64::new(0),
            tcp_open_failures: AtomicU64::new(0),
            last_tcp_open_latency_ms: AtomicU64::new(0),
            ssh_session_connect_attempts: AtomicU64::new(0),
            ssh_session_connect_failures: AtomicU64::new(0),
            ssh_channel_open_attempts: AtomicU64::new(0),
            ssh_channel_open_failures: AtomicU64::new(0),
            active_ssh_channels: AtomicU64::new(0),
            total_tcp: AtomicU64::new(0),
            active_tcp: AtomicU64::new(0),
            bytes_client_to_remote: AtomicU64::new(0),
            bytes_remote_to_client: AtomicU64::new(0),
            first_byte_samples: AtomicU64::new(0),
            first_byte_latency_total_ms: AtomicU64::new(0),
            last_first_byte_latency_ms: AtomicU64::new(0),
            max_first_byte_latency_ms: AtomicU64::new(0),
            first_byte_latency_buckets: empty_latency_buckets(),
            graceful_closes: AtomicU64::new(0),
            error_closes: AtomicU64::new(0),
            last_close_reason: StdMutex::new(None),
            session_growth_events: AtomicU64::new(0),
            session_growth_suppressed: AtomicU64::new(0),
            last_channel_queue_depth: AtomicU64::new(0),
            max_channel_queue_depth: AtomicU64::new(0),
            last_error: Mutex::new(None),
        })
    }

    pub async fn open_stream(self: &Arc<Self>, host: String, port: u16) -> Result<Stream> {
        let session = self.session().await?;

        let started = Instant::now();
        let opened = time::timeout(
            Duration::from_secs(self.args.connect_timeout_secs.max(1)),
            session.client.direct_tcpip_stream(host.clone(), port),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "ssh-native direct-tcpip to {host}:{port} timed out after {}s",
                self.args.connect_timeout_secs.max(1)
            )
        })
        .and_then(|result| result);

        match opened {
            Ok(inner) => {
                session.opened_channels.fetch_add(1, Ordering::Relaxed);
                session
                    .last_open_latency_ms
                    .store(duration_millis(started.elapsed()), Ordering::Relaxed);
                *self.last_error.lock().await = None;
                Ok(Stream {
                    inner,
                    state: self.clone(),
                    session,
                    opened_at: Instant::now(),
                    first_byte_recorded: false,
                    close_recorded: false,
                })
            }
            Err(err) => {
                session.active_channels.fetch_sub(1, Ordering::Relaxed);
                self.active_ssh_channels.fetch_sub(1, Ordering::Relaxed);
                self.ssh_channel_open_failures
                    .fetch_add(1, Ordering::Relaxed);
                session.open_failures.fetch_add(1, Ordering::Relaxed);
                session.last_failure_elapsed_ms.store(
                    duration_millis(self.started.elapsed()).saturating_add(1),
                    Ordering::Relaxed,
                );
                *self.last_error.lock().await = Some(err.to_string());
                Err(err)
            }
        }
    }

    async fn session(self: &Arc<Self>) -> Result<Arc<Session>> {
        let mut sessions = self.sessions.lock().await;
        let min_active_channels = sessions
            .iter()
            .map(|session| session.active_channels.load(Ordering::Relaxed))
            .min()
            .unwrap_or(0);
        let all_sessions_busy = !sessions.is_empty() && min_active_channels > 0;
        if sessions.len() > 0
            && sessions.len() < self.pool_size
            && all_sessions_busy
            && min_active_channels < SSH_SESSION_GROWTH_MIN_ACTIVE_CHANNELS
        {
            self.session_growth_suppressed
                .fetch_add(1, Ordering::Relaxed);
        }
        if should_open_new_session(sessions.len(), self.pool_size, min_active_channels) {
            self.ssh_session_connect_attempts
                .fetch_add(1, Ordering::Relaxed);
            let id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
            let client = match ssh_client::Client::connect_proxy_args(&self.args).await {
                Ok(client) => client,
                Err(err) => {
                    self.ssh_session_connect_failures
                        .fetch_add(1, Ordering::Relaxed);
                    *self.last_error.lock().await = Some(err.to_string());
                    return Err(err);
                }
            };
            let session = Arc::new(Session {
                id,
                client,
                active_channels: AtomicU32::new(0),
                opened_channels: AtomicU64::new(0),
                open_failures: AtomicU64::new(0),
                last_open_latency_ms: AtomicU64::new(0),
                last_failure_elapsed_ms: AtomicU64::new(0),
                bytes_client_to_remote: AtomicU64::new(0),
                bytes_remote_to_client: AtomicU64::new(0),
                first_byte_samples: AtomicU64::new(0),
                first_byte_latency_total_ms: AtomicU64::new(0),
                last_first_byte_latency_ms: AtomicU64::new(0),
                max_first_byte_latency_ms: AtomicU64::new(0),
                first_byte_latency_buckets: empty_latency_buckets(),
                graceful_closes: AtomicU64::new(0),
                error_closes: AtomicU64::new(0),
                last_close_reason: StdMutex::new(None),
                last_channel_queue_depth: AtomicU64::new(0),
                max_channel_queue_depth: AtomicU64::new(0),
            });
            sessions.push(session.clone());
            self.session_growth_events.fetch_add(1, Ordering::Relaxed);
            self.acquire_session_channel(&session);
            return Ok(session);
        }

        let now_ms = duration_millis(self.started.elapsed());
        let session = sessions
            .iter()
            .min_by_key(|session| self.session_score(session, now_ms))
            .cloned()
            .ok_or_else(|| anyhow!("ssh-native session pool is empty"))?;
        self.acquire_session_channel(&session);
        Ok(session)
    }

    fn acquire_session_channel(&self, session: &Session) {
        let queue_depth = session.active_channels.fetch_add(1, Ordering::Relaxed) as u64;
        self.active_ssh_channels.fetch_add(1, Ordering::Relaxed);
        self.ssh_channel_open_attempts
            .fetch_add(1, Ordering::Relaxed);
        session
            .last_channel_queue_depth
            .store(queue_depth, Ordering::Relaxed);
        update_atomic_max(&session.max_channel_queue_depth, queue_depth);
        self.last_channel_queue_depth
            .store(queue_depth, Ordering::Relaxed);
        update_atomic_max(&self.max_channel_queue_depth, queue_depth);
    }

    fn session_score(&self, session: &Session, now_ms: u64) -> u64 {
        self.session_score_components(session, now_ms).score()
    }

    fn session_score_components(&self, session: &Session, now_ms: u64) -> SessionScoreComponents {
        let active = session.active_channels.load(Ordering::Relaxed) as u64;
        let failures = session.open_failures.load(Ordering::Relaxed);
        let latency = session.last_open_latency_ms.load(Ordering::Relaxed);
        let first_byte_latency = last_sampled_u64(
            session.first_byte_samples.load(Ordering::Relaxed),
            session.last_first_byte_latency_ms.load(Ordering::Relaxed),
        )
        .unwrap_or(0);
        let error_closes = session.error_closes.load(Ordering::Relaxed);
        let bytes = session
            .bytes_client_to_remote
            .load(Ordering::Relaxed)
            .saturating_add(session.bytes_remote_to_client.load(Ordering::Relaxed));
        let recent_failure_penalty = session
            .failure_age_ms(now_ms)
            .filter(|age| *age < RECENT_FAILURE_WINDOW_MS)
            .map(|age| {
                RECENT_FAILURE_WINDOW_MS
                    .saturating_sub(age)
                    .saturating_add(10_000)
            })
            .unwrap_or(0);

        SessionScoreComponents {
            active_channels: active,
            open_failures: failures,
            last_open_latency_ms: latency,
            first_byte_latency_ms: first_byte_latency,
            bytes_in_flight: bytes,
            recent_failure_penalty,
            error_closes,
        }
    }

    pub fn record_tcp_open(&self) {
        self.total_tcp.fetch_add(1, Ordering::Relaxed);
        self.active_tcp.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_tcp_close(&self) {
        self.active_tcp.fetch_sub(1, Ordering::Relaxed);
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

    pub async fn status_value(&self) -> serde_json::Value {
        let sessions = self.sessions.lock().await;
        let now_ms = duration_millis(self.started.elapsed());
        let active_ssh_channels = self.active_ssh_channels.load(Ordering::Relaxed);
        let ssh_channel_open_attempts = self.ssh_channel_open_attempts.load(Ordering::Relaxed);
        let ssh_channel_open_failures = self.ssh_channel_open_failures.load(Ordering::Relaxed);
        let bytes_client_to_remote = self.bytes_client_to_remote.load(Ordering::Relaxed);
        let bytes_remote_to_client = self.bytes_remote_to_client.load(Ordering::Relaxed);
        let first_byte_samples = self.first_byte_samples.load(Ordering::Relaxed);
        let last_first_byte_latency_ms = last_sampled_u64(
            first_byte_samples,
            self.last_first_byte_latency_ms.load(Ordering::Relaxed),
        );
        let last_close_reason = self
            .last_close_reason
            .lock()
            .ok()
            .and_then(|reason| reason.clone());
        let last_error = self.last_error.lock().await.clone();
        let workers = sessions
            .iter()
            .map(|session| {
                let score_components = self.session_score_components(session, now_ms);
                serde_json::json!({
                    "session_id": session.id,
                    "active_channels": session.active_channels.load(Ordering::Relaxed),
                    "opened_channels": session.opened_channels.load(Ordering::Relaxed),
                    "open_failures": session.open_failures.load(Ordering::Relaxed),
                    "last_open_latency_ms": last_sampled_u64(
                        session.opened_channels.load(Ordering::Relaxed),
                        session.last_open_latency_ms.load(Ordering::Relaxed),
                    ),
                    "last_failure_age_ms": session.failure_age_ms(now_ms),
                    "bytes_client_to_remote": session.bytes_client_to_remote.load(Ordering::Relaxed),
                    "bytes_remote_to_client": session.bytes_remote_to_client.load(Ordering::Relaxed),
                    "first_byte_samples": session.first_byte_samples.load(Ordering::Relaxed),
                    "avg_first_byte_latency_ms": average_sampled_u64(
                        session.first_byte_samples.load(Ordering::Relaxed),
                        session.first_byte_latency_total_ms.load(Ordering::Relaxed),
                    ),
                    "last_first_byte_latency_ms": last_sampled_u64(
                        session.first_byte_samples.load(Ordering::Relaxed),
                        session.last_first_byte_latency_ms.load(Ordering::Relaxed),
                    ),
                    "max_first_byte_latency_ms": last_sampled_u64(
                        session.first_byte_samples.load(Ordering::Relaxed),
                        session.max_first_byte_latency_ms.load(Ordering::Relaxed),
                    ),
                    "p50_first_byte_latency_ms": latency_percentile(
                        &session.first_byte_latency_buckets,
                        session.first_byte_samples.load(Ordering::Relaxed),
                        50,
                    ),
                    "p95_first_byte_latency_ms": latency_percentile(
                        &session.first_byte_latency_buckets,
                        session.first_byte_samples.load(Ordering::Relaxed),
                        95,
                    ),
                    "graceful_closes": session.graceful_closes.load(Ordering::Relaxed),
                    "error_closes": session.error_closes.load(Ordering::Relaxed),
                    "last_close_reason": session.last_close_reason.lock().ok().and_then(|reason| reason.clone()),
                    "last_channel_queue_depth": session.last_channel_queue_depth.load(Ordering::Relaxed),
                    "max_channel_queue_depth": session.max_channel_queue_depth.load(Ordering::Relaxed),
                    "score": score_components.score(),
                    "score_components": score_components.to_json(),
                })
            })
            .collect::<Vec<_>>();
        let mut status = serde_json::json!({
            "connected": !sessions.is_empty(),
            "selected_protocol": "ssh-native",
            "ssh_mode": "native-direct-tcpip",
            "ssh_session_pool_size": self.pool_size,
            "ssh_session_pool_source": self.args.ssh_session_pool_source.as_deref().unwrap_or("implicit"),
            "ssh_session_pool_reason": self.args.ssh_session_pool_reason.as_deref().unwrap_or_else(|| {
                if self.args.tcp_target.is_some() {
                    "implicit ssh-native single-session default for fixed --tcp-target routes"
                } else {
                    "implicit ssh-native two-session default for multi-flow SOCKS/HTTP proxy routes"
                }
            }),
            "ssh_session_pool_warning": self.args.ssh_session_pool_warning.as_deref(),
            "ssh_session_growth_active_threshold": SSH_SESSION_GROWTH_MIN_ACTIVE_CHANNELS,
            "ssh_session_growth_events": self.session_growth_events.load(Ordering::Relaxed),
            "ssh_session_growth_suppressed": self.session_growth_suppressed.load(Ordering::Relaxed),
            "active_ssh_sessions": sessions.len(),
            "active_ssh_channels": self.active_ssh_channels.load(Ordering::Relaxed),
            "ssh_session_connect_attempts": self.ssh_session_connect_attempts.load(Ordering::Relaxed),
            "ssh_session_connect_failures": self.ssh_session_connect_failures.load(Ordering::Relaxed),
            "ssh_channel_open_attempts": ssh_channel_open_attempts,
            "ssh_channel_open_failures": ssh_channel_open_failures,
            "workers": workers,
            "uptime_secs": self.started.elapsed().as_secs(),
            "active_tcp": self.active_tcp.load(Ordering::Relaxed),
            "total_tcp": self.total_tcp.load(Ordering::Relaxed),
            "tcp_open_attempts": self.tcp_open_attempts.load(Ordering::Relaxed),
            "tcp_open_successes": self.tcp_open_successes.load(Ordering::Relaxed),
            "tcp_open_failures": self.tcp_open_failures.load(Ordering::Relaxed),
            "last_tcp_open_latency_ms": last_sampled_u64(
                self.tcp_open_attempts.load(Ordering::Relaxed),
                self.last_tcp_open_latency_ms.load(Ordering::Relaxed),
            ),
            "bytes_client_to_remote": bytes_client_to_remote,
            "bytes_remote_to_client": bytes_remote_to_client,
            "first_byte_samples": first_byte_samples,
            "avg_first_byte_latency_ms": average_sampled_u64(
                first_byte_samples,
                self.first_byte_latency_total_ms.load(Ordering::Relaxed),
            ),
            "last_first_byte_latency_ms": last_first_byte_latency_ms,
            "max_first_byte_latency_ms": last_sampled_u64(
                first_byte_samples,
                self.max_first_byte_latency_ms.load(Ordering::Relaxed),
            ),
            "p50_first_byte_latency_ms": latency_percentile(
                &self.first_byte_latency_buckets,
                first_byte_samples,
                50,
            ),
            "p95_first_byte_latency_ms": latency_percentile(
                &self.first_byte_latency_buckets,
                first_byte_samples,
                95,
            ),
            "graceful_closes": self.graceful_closes.load(Ordering::Relaxed),
            "error_closes": self.error_closes.load(Ordering::Relaxed),
            "last_close_reason": last_close_reason,
            "last_channel_queue_depth": self.last_channel_queue_depth.load(Ordering::Relaxed),
            "max_channel_queue_depth": self.max_channel_queue_depth.load(Ordering::Relaxed),
            "read_buffer_size": protocol::TCP_DATA_CHUNK,
            "write_batch_limit": protocol::FRAME_WRITE_BATCH_LIMIT,
            "frame_channel_capacity": protocol::FRAME_CHANNEL_CAPACITY,
            "last_error": last_error,
        });
        if let Some(object) = status.as_object_mut() {
            object.insert(
                "ssh_session_scheduler".to_string(),
                serde_json::json!({
                    "policy": "warm-primary until active-channel pressure reaches growth threshold; otherwise choose the lowest scored healthy session",
                    "growth_active_threshold": SSH_SESSION_GROWTH_MIN_ACTIVE_CHANNELS,
                    "score_components": [
                        "active_channels",
                        "open_failures",
                        "last_open_latency_ms",
                        "first_byte_latency_ms",
                        "bytes_in_flight",
                        "recent_failure_penalty",
                        "error_closes",
                    ],
                }),
            );
            object.insert(
                "link".to_string(),
                serde_json::json!({
                    "health": {
                        "selected_protocol": "ssh-native",
                        "active_connections": sessions.len(),
                        "active_streams": active_ssh_channels,
                        "active_channels": active_ssh_channels,
                        "pool_size": self.pool_size,
                        "pool_policy": self.args.pool_policy.as_deref(),
                        "pool_workload_hint": self.args.workload_hint.map(workload_hint_name),
                        "open_attempts": ssh_channel_open_attempts,
                        "open_successes": ssh_channel_open_attempts.saturating_sub(ssh_channel_open_failures),
                        "open_failures": ssh_channel_open_failures,
                        "open_latency_ms": last_sampled_u64(
                            ssh_channel_open_attempts,
                            self.last_tcp_open_latency_ms.load(Ordering::Relaxed),
                        ),
                        "bytes_client_to_remote": bytes_client_to_remote,
                        "bytes_remote_to_client": bytes_remote_to_client,
                        "first_byte_samples": first_byte_samples,
                        "first_byte_latency_ms": last_first_byte_latency_ms,
                        "avg_first_byte_latency_ms": average_sampled_u64(
                            first_byte_samples,
                            self.first_byte_latency_total_ms.load(Ordering::Relaxed),
                        ),
                        "max_first_byte_latency_ms": last_sampled_u64(
                            first_byte_samples,
                            self.max_first_byte_latency_ms.load(Ordering::Relaxed),
                        ),
                        "p50_first_byte_latency_ms": latency_percentile(
                            &self.first_byte_latency_buckets,
                            first_byte_samples,
                            50,
                        ),
                        "p95_first_byte_latency_ms": latency_percentile(
                            &self.first_byte_latency_buckets,
                            first_byte_samples,
                            95,
                        ),
                        "last_close_reason": last_close_reason,
                        "degraded_reason": last_error,
                        "healthy_workers": sessions.len(),
                        "degraded_workers": if last_error.is_some() { 1 } else { 0 },
                        "reconnecting_workers": 0,
                        "control_health": if sessions.is_empty() {
                            "disconnected"
                        } else if last_error.is_some() {
                            "degraded"
                        } else {
                            "healthy"
                        },
                        "connected": !sessions.is_empty(),
                    }
                }),
            );
        }
        status
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

    async fn shutdown_notified(&self) {
        if self.shutdown.load(Ordering::Relaxed) {
            return;
        }
        self.shutdown_notify.notified().await;
    }
}

const RECENT_FAILURE_WINDOW_MS: u64 = 30_000;
const SSH_SESSION_GROWTH_MIN_ACTIVE_CHANNELS: u32 = 2;
const FIRST_BYTE_LATENCY_BUCKETS: [u64; 8] = [10, 25, 50, 100, 250, 500, 1_000, 5_000];
const FIRST_BYTE_LATENCY_BUCKET_COUNT: usize = FIRST_BYTE_LATENCY_BUCKETS.len() + 1;

#[derive(Debug, Clone, Copy)]
struct SessionScoreComponents {
    active_channels: u64,
    open_failures: u64,
    last_open_latency_ms: u64,
    first_byte_latency_ms: u64,
    bytes_in_flight: u64,
    recent_failure_penalty: u64,
    error_closes: u64,
}

impl SessionScoreComponents {
    fn score(self) -> u64 {
        calculate_session_score(self)
    }

    fn to_json(self) -> serde_json::Value {
        serde_json::json!({
            "active_channels": self.active_channels,
            "open_failures": self.open_failures,
            "last_open_latency_ms": self.last_open_latency_ms,
            "first_byte_latency_ms": self.first_byte_latency_ms,
            "bytes_in_flight": self.bytes_in_flight,
            "recent_failure_penalty": self.recent_failure_penalty,
            "error_closes": self.error_closes,
        })
    }
}

fn should_open_new_session(
    existing_sessions: usize,
    pool_size: usize,
    min_active_channels: u32,
) -> bool {
    existing_sessions == 0
        || (existing_sessions < pool_size
            && min_active_channels >= SSH_SESSION_GROWTH_MIN_ACTIVE_CHANNELS)
}

fn calculate_session_score(components: SessionScoreComponents) -> u64 {
    components
        .active_channels
        .saturating_mul(10_000)
        .saturating_add(components.open_failures.saturating_mul(1_000))
        .saturating_add(components.error_closes.saturating_mul(750))
        .saturating_add(components.last_open_latency_ms.min(5_000))
        .saturating_add(components.first_byte_latency_ms.min(5_000))
        .saturating_add(components.bytes_in_flight / (1024 * 1024))
        .saturating_add(components.recent_failure_penalty)
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn last_sampled_u64(sample_count: u64, value: u64) -> Option<u64> {
    (sample_count > 0).then_some(value)
}

fn average_sampled_u64(sample_count: u64, total: u64) -> Option<u64> {
    if sample_count == 0 {
        None
    } else {
        Some(total / sample_count)
    }
}

fn workload_hint_name(hint: cli::RouteWorkloadHint) -> &'static str {
    match hint {
        cli::RouteWorkloadHint::Large => "large",
        cli::RouteWorkloadHint::Concurrent => "concurrent",
        cli::RouteWorkloadHint::Mixed => "mixed",
    }
}

fn empty_latency_buckets() -> [AtomicU64; FIRST_BYTE_LATENCY_BUCKET_COUNT] {
    std::array::from_fn(|_| AtomicU64::new(0))
}

fn record_latency_sample(
    samples: &AtomicU64,
    total: &AtomicU64,
    last: &AtomicU64,
    max: &AtomicU64,
    buckets: &[AtomicU64; FIRST_BYTE_LATENCY_BUCKET_COUNT],
    latency_ms: u64,
) {
    samples.fetch_add(1, Ordering::Relaxed);
    total.fetch_add(latency_ms, Ordering::Relaxed);
    last.store(latency_ms, Ordering::Relaxed);
    update_atomic_max(max, latency_ms);
    let bucket = FIRST_BYTE_LATENCY_BUCKETS
        .iter()
        .position(|limit| latency_ms <= *limit)
        .unwrap_or(FIRST_BYTE_LATENCY_BUCKETS.len());
    buckets[bucket].fetch_add(1, Ordering::Relaxed);
}

fn latency_percentile(
    buckets: &[AtomicU64; FIRST_BYTE_LATENCY_BUCKET_COUNT],
    sample_count: u64,
    percentile: u64,
) -> Option<u64> {
    if sample_count == 0 {
        return None;
    }
    let target = sample_count.saturating_mul(percentile).div_ceil(100);
    let mut seen = 0_u64;
    for (index, bucket) in buckets.iter().enumerate() {
        seen = seen.saturating_add(bucket.load(Ordering::Relaxed));
        if seen >= target {
            return Some(
                FIRST_BYTE_LATENCY_BUCKETS
                    .get(index)
                    .copied()
                    .unwrap_or(u64::MAX),
            );
        }
    }
    Some(u64::MAX)
}

fn update_atomic_max(value: &AtomicU64, candidate: u64) {
    let mut current = value.load(Ordering::Relaxed);
    while candidate > current {
        match value.compare_exchange_weak(current, candidate, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}

impl Session {
    fn failure_age_ms(&self, now_ms: u64) -> Option<u64> {
        let failed_at = self.last_failure_elapsed_ms.load(Ordering::Relaxed);
        (failed_at > 0).then(|| now_ms.saturating_sub(failed_at.saturating_sub(1)))
    }
}

impl Stream {
    fn record_first_byte_latency(&self, latency: Duration) {
        let latency_ms = duration_millis(latency);
        record_latency_sample(
            &self.state.first_byte_samples,
            &self.state.first_byte_latency_total_ms,
            &self.state.last_first_byte_latency_ms,
            &self.state.max_first_byte_latency_ms,
            &self.state.first_byte_latency_buckets,
            latency_ms,
        );
        record_latency_sample(
            &self.session.first_byte_samples,
            &self.session.first_byte_latency_total_ms,
            &self.session.last_first_byte_latency_ms,
            &self.session.max_first_byte_latency_ms,
            &self.session.first_byte_latency_buckets,
            latency_ms,
        );
    }

    pub fn record_client_to_remote_bytes(&self, bytes: usize) {
        self.state.record_client_to_remote_bytes(bytes);
        self.session
            .bytes_client_to_remote
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    pub fn record_remote_to_client_bytes(&self, bytes: usize) {
        self.state.record_remote_to_client_bytes(bytes);
        self.session
            .bytes_remote_to_client
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    pub fn record_graceful_close(&mut self, reason: impl Into<String>) {
        self.record_close(CloseKind::Graceful, reason.into());
    }

    pub fn record_error_close(&mut self, reason: impl Into<String>) {
        self.record_close(CloseKind::Error, reason.into());
    }

    fn record_close(&mut self, kind: CloseKind, reason: String) {
        if self.close_recorded {
            return;
        }
        self.close_recorded = true;
        match kind {
            CloseKind::Graceful => {
                self.state.graceful_closes.fetch_add(1, Ordering::Relaxed);
                self.session.graceful_closes.fetch_add(1, Ordering::Relaxed);
            }
            CloseKind::Error => {
                self.state.error_closes.fetch_add(1, Ordering::Relaxed);
                self.session.error_closes.fetch_add(1, Ordering::Relaxed);
            }
        }
        if let Ok(mut last) = self.state.last_close_reason.lock() {
            *last = Some(reason.clone());
        }
        if let Ok(mut last) = self.session.last_close_reason.lock() {
            *last = Some(reason);
        }
    }
}

enum CloseKind {
    Graceful,
    Error,
}

impl Drop for Stream {
    fn drop(&mut self) {
        if !self.close_recorded {
            self.record_error_close("dropped before relay recorded close");
        }
        self.session.active_channels.fetch_sub(1, Ordering::Relaxed);
        self.state
            .active_ssh_channels
            .fetch_sub(1, Ordering::Relaxed);
    }
}

impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &result {
            let after = buf.filled().len();
            if after > before && !self.first_byte_recorded {
                self.first_byte_recorded = true;
                self.record_first_byte_latency(self.opened_at.elapsed());
            }
        }
        result
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub async fn run_with_state(args: cli::ProxyArgs, state: Arc<State>) -> Result<()> {
    if let Some(addr) = args.control_listen {
        let control_state = state.clone();
        tokio::spawn(async move {
            if let Err(err) = run_control_server(addr, control_state).await {
                warn!(%addr, error = %err, "ssh-native control server stopped");
            }
        });
    }

    let listener = TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("failed to bind ssh-native proxy listener {}", args.listen))?;
    info!(listen = %args.listen, "ssh-native proxy listening");

    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, peer) = accept?;
                let state = state.clone();
                let tcp_target = args.tcp_target.clone();
                tokio::spawn(async move {
                    let result = if let Some(target) = tcp_target {
                        socks::handle_fixed_target_ssh_native(stream, peer, target, state).await
                    } else {
                        socks::handle_client_ssh_native(stream, peer, state).await
                    };
                    if let Err(err) = result {
                        debug!(%peer, error = %err, "ssh-native proxy client failed");
                    }
                });
            }
            _ = state.shutdown_notified() => {
                info!("shutdown requested; stopping ssh-native proxy listener");
                break;
            }
        }
    }
    Ok(())
}

async fn run_control_server(addr: SocketAddr, state: Arc<State>) -> Result<()> {
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind ssh-native control listener {addr}"))?;
    info!(%addr, "ssh-native control listener ready");
    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, peer) = accept?;
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_control(stream, state).await {
                        warn!(%peer, error = %err, "ssh-native control request failed");
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

    fn proxy_args() -> cli::ProxyArgs {
        cli::ProxyArgs {
            target: "peer".to_string(),
            listen: "127.0.0.1:18080".parse().unwrap(),
            tcp_target: None,
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
            deploy: cli::DeployMode::Auto,
            remote_os: cli::RemoteOs::Auto,
            remote_transport: cli::RemoteTransport::SshNative,
            remote_tcp: "127.0.0.1:19080".parse().unwrap(),
            remote_control: "127.0.0.1:19081".parse().unwrap(),
            remote_quic: None,
            allow_plain_tcp: false,
            remote_tls: None,
            remote_ca: None,
            remote_name: "localhost".to_string(),
            remote_client_cert: None,
            remote_client_key: None,
            remote_token: None,
            egress_proxy: None,
            reconnect_delay_secs: 5,
            reconnect_max_delay_secs: 60,
            connect_timeout_secs: 30,
            transport_pool_size: 1,
            pool_policy: None,
            workload_hint: None,
            quic_max_bidi_streams: crate::peer_transport::QUIC_MAX_BIDI_STREAMS,
            quic_stream_receive_window: crate::peer_transport::QUIC_STREAM_RECEIVE_WINDOW,
            quic_receive_window: crate::peer_transport::QUIC_RECEIVE_WINDOW,
            quic_keep_alive_interval_secs: crate::peer_transport::QUIC_KEEP_ALIVE_INTERVAL_SECS,
            quic_idle_timeout_secs: crate::peer_transport::QUIC_IDLE_TIMEOUT_SECS,
            ssh_session_pool_size: Some(2),
            ssh_session_pool_source: Some("implicit".to_string()),
            ssh_session_pool_reason: Some(
                "implicit ssh-native two-session default for multi-flow SOCKS/HTTP proxy routes"
                    .to_string(),
            ),
            ssh_session_pool_warning: None,
            transport_pool_source: Some("implicit".to_string()),
            transport_pool_reason: Some("test pool".to_string()),
            transport_selection_source: Some("topology".to_string()),
            transport_selection_reason: Some("test selection".to_string()),
            preflight_recommended_fallback: None,
            preflight_selected_reason: None,
            preflight_repair_hint: None,
            preflight_candidate_failures: Vec::new(),
            no_reconnect: false,
            control_listen: None,
        }
    }

    #[test]
    fn ssh_session_pool_keeps_warm_primary_until_busy() {
        assert!(should_open_new_session(0, 4, 0));
        assert!(!should_open_new_session(1, 4, 0));
        assert!(!should_open_new_session(1, 4, 1));
        assert!(should_open_new_session(1, 4, 2));
        assert!(!should_open_new_session(4, 4, 8));
    }

    #[test]
    fn ssh_session_score_penalizes_busy_failed_and_slow_sessions() {
        let idle = calculate_session_score(SessionScoreComponents {
            active_channels: 0,
            open_failures: 0,
            last_open_latency_ms: 10,
            first_byte_latency_ms: 10,
            bytes_in_flight: 0,
            recent_failure_penalty: 0,
            error_closes: 0,
        });
        let busy = calculate_session_score(SessionScoreComponents {
            active_channels: 1,
            open_failures: 0,
            last_open_latency_ms: 10,
            first_byte_latency_ms: 10,
            bytes_in_flight: 0,
            recent_failure_penalty: 0,
            error_closes: 0,
        });
        let failed = calculate_session_score(SessionScoreComponents {
            active_channels: 0,
            open_failures: 1,
            last_open_latency_ms: 10,
            first_byte_latency_ms: 10,
            bytes_in_flight: 0,
            recent_failure_penalty: 20_000,
            error_closes: 0,
        });
        let loaded = calculate_session_score(SessionScoreComponents {
            active_channels: 0,
            open_failures: 0,
            last_open_latency_ms: 10,
            first_byte_latency_ms: 10,
            bytes_in_flight: 32 * 1024 * 1024,
            recent_failure_penalty: 0,
            error_closes: 0,
        });
        let slow_first_byte = calculate_session_score(SessionScoreComponents {
            active_channels: 0,
            open_failures: 0,
            last_open_latency_ms: 10,
            first_byte_latency_ms: 2_000,
            bytes_in_flight: 0,
            recent_failure_penalty: 0,
            error_closes: 0,
        });
        let error_closed = calculate_session_score(SessionScoreComponents {
            active_channels: 0,
            open_failures: 0,
            last_open_latency_ms: 10,
            first_byte_latency_ms: 10,
            bytes_in_flight: 0,
            recent_failure_penalty: 0,
            error_closes: 2,
        });

        assert!(idle < busy);
        assert!(idle < failed);
        assert!(idle < loaded);
        assert!(idle < slow_first_byte);
        assert!(idle < error_closed);
    }

    #[tokio::test]
    async fn ssh_status_exposes_first_byte_and_close_metrics() {
        let state = State::new(proxy_args());

        let status = state.status_value().await;

        assert_eq!(status["first_byte_samples"], 0);
        assert_eq!(
            status["last_first_byte_latency_ms"],
            serde_json::Value::Null
        );
        assert_eq!(status["graceful_closes"], 0);
        assert_eq!(status["error_closes"], 0);
        assert_eq!(status["last_close_reason"], serde_json::Value::Null);
        assert_eq!(status["avg_first_byte_latency_ms"], serde_json::Value::Null);
        assert_eq!(status["max_first_byte_latency_ms"], serde_json::Value::Null);
        assert_eq!(status["p50_first_byte_latency_ms"], serde_json::Value::Null);
        assert_eq!(status["p95_first_byte_latency_ms"], serde_json::Value::Null);
        assert_eq!(status["ssh_session_growth_active_threshold"], 2);
        assert_eq!(status["ssh_session_growth_events"], 0);
        assert_eq!(status["ssh_session_growth_suppressed"], 0);
        assert_eq!(
            status["ssh_session_scheduler"]["growth_active_threshold"],
            2
        );
        assert!(
            status["ssh_session_scheduler"]["score_components"]
                .as_array()
                .expect("score components")
                .contains(&serde_json::json!("first_byte_latency_ms"))
        );
        assert_eq!(status["max_channel_queue_depth"], 0);
        assert_eq!(status["link"]["health"]["selected_protocol"], "ssh-native");
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
