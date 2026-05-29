use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use serde::Serialize;
use tokio::{
    io::{self, AsyncRead, AsyncWrite},
    sync::{Mutex, RwLock, mpsc, oneshot},
    task::JoinHandle,
    time::{self, Duration},
};
use tracing::{debug, info, warn};

use crate::{
    cli, deploy, peer_transport,
    protocol::{
        FRAME_CHANNEL_CAPACITY, Frame, FrameReader, FrameWriteBatchStats,
        TCP_STREAM_BACKPRESSURE_TIMEOUT, TCP_STREAM_CHANNEL_CAPACITY,
        UDP_ASSOC_BACKPRESSURE_TIMEOUT, UdpDatagram, write_frame_batch,
    },
};

const OPEN_TCP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct BridgeHandle {
    inner: Arc<BridgeInner>,
}

struct BridgeInner {
    writer: mpsc::Sender<Frame>,
    next_id: AtomicU32,
    pending_tcp: Mutex<HashMap<u32, oneshot::Sender<Result<(), String>>>>,
    tcp_streams: RwLock<HashMap<u32, mpsc::Sender<Bytes>>>,
    udp_streams: RwLock<HashMap<u32, mpsc::Sender<UdpDatagram>>>,
    metrics: Arc<BridgeMetrics>,
}

pub struct Bridge {
    pub handle: BridgeHandle,
    pub lifecycle: JoinHandle<()>,
    pub selected_protocol: Option<peer_transport::PeerProtocol>,
    pub transport_timings: deploy::RemoteHelperTimings,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BridgeMetricsSnapshot {
    pub frame_write_batches: u64,
    pub frame_write_flushes: u64,
    pub frame_write_frames: u64,
    pub frame_write_data_frames: u64,
    pub frame_write_data_bytes: u64,
    pub frame_write_vectored_writes: u64,
    pub frame_write_failures: u64,
    pub frame_read_frames: u64,
    pub frame_read_data_frames: u64,
    pub frame_read_data_bytes: u64,
    pub tcp_stream_backpressure_timeouts: u64,
    pub udp_assoc_backpressure_timeouts: u64,
}

impl BridgeMetricsSnapshot {
    pub fn merge(&mut self, other: &Self) {
        self.frame_write_batches += other.frame_write_batches;
        self.frame_write_flushes += other.frame_write_flushes;
        self.frame_write_frames += other.frame_write_frames;
        self.frame_write_data_frames += other.frame_write_data_frames;
        self.frame_write_data_bytes += other.frame_write_data_bytes;
        self.frame_write_vectored_writes += other.frame_write_vectored_writes;
        self.frame_write_failures += other.frame_write_failures;
        self.frame_read_frames += other.frame_read_frames;
        self.frame_read_data_frames += other.frame_read_data_frames;
        self.frame_read_data_bytes += other.frame_read_data_bytes;
        self.tcp_stream_backpressure_timeouts += other.tcp_stream_backpressure_timeouts;
        self.udp_assoc_backpressure_timeouts += other.udp_assoc_backpressure_timeouts;
    }
}

#[derive(Default)]
struct BridgeMetrics {
    frame_write_batches: AtomicU64,
    frame_write_flushes: AtomicU64,
    frame_write_frames: AtomicU64,
    frame_write_data_frames: AtomicU64,
    frame_write_data_bytes: AtomicU64,
    frame_write_vectored_writes: AtomicU64,
    frame_write_failures: AtomicU64,
    frame_read_frames: AtomicU64,
    frame_read_data_frames: AtomicU64,
    frame_read_data_bytes: AtomicU64,
    tcp_stream_backpressure_timeouts: AtomicU64,
    udp_assoc_backpressure_timeouts: AtomicU64,
}

impl BridgeMetrics {
    fn record_write_batch(&self, stats: &FrameWriteBatchStats) {
        self.frame_write_batches.fetch_add(1, Ordering::Relaxed);
        self.frame_write_flushes
            .fetch_add(stats.flushes as u64, Ordering::Relaxed);
        self.frame_write_frames
            .fetch_add(stats.frames_written as u64, Ordering::Relaxed);
        self.frame_write_data_frames
            .fetch_add(stats.data_frames_written as u64, Ordering::Relaxed);
        self.frame_write_data_bytes
            .fetch_add(stats.data_bytes_written as u64, Ordering::Relaxed);
        self.frame_write_vectored_writes
            .fetch_add(stats.vectored_writes as u64, Ordering::Relaxed);
    }

    fn record_write_failure(&self) {
        self.frame_write_failures.fetch_add(1, Ordering::Relaxed);
    }

    fn record_read_frame(&self, frame: &Frame) {
        self.frame_read_frames.fetch_add(1, Ordering::Relaxed);
        if let Frame::Data { data, .. } = frame {
            self.frame_read_data_frames.fetch_add(1, Ordering::Relaxed);
            self.frame_read_data_bytes
                .fetch_add(data.len() as u64, Ordering::Relaxed);
        }
    }

    fn record_tcp_backpressure_timeout(&self) {
        self.tcp_stream_backpressure_timeouts
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_udp_backpressure_timeout(&self) {
        self.udp_assoc_backpressure_timeouts
            .fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> BridgeMetricsSnapshot {
        BridgeMetricsSnapshot {
            frame_write_batches: self.frame_write_batches.load(Ordering::Relaxed),
            frame_write_flushes: self.frame_write_flushes.load(Ordering::Relaxed),
            frame_write_frames: self.frame_write_frames.load(Ordering::Relaxed),
            frame_write_data_frames: self.frame_write_data_frames.load(Ordering::Relaxed),
            frame_write_data_bytes: self.frame_write_data_bytes.load(Ordering::Relaxed),
            frame_write_vectored_writes: self.frame_write_vectored_writes.load(Ordering::Relaxed),
            frame_write_failures: self.frame_write_failures.load(Ordering::Relaxed),
            frame_read_frames: self.frame_read_frames.load(Ordering::Relaxed),
            frame_read_data_frames: self.frame_read_data_frames.load(Ordering::Relaxed),
            frame_read_data_bytes: self.frame_read_data_bytes.load(Ordering::Relaxed),
            tcp_stream_backpressure_timeouts: self
                .tcp_stream_backpressure_timeouts
                .load(Ordering::Relaxed),
            udp_assoc_backpressure_timeouts: self
                .udp_assoc_backpressure_timeouts
                .load(Ordering::Relaxed),
        }
    }
}

impl BridgeHandle {
    pub fn metrics_snapshot(&self) -> BridgeMetricsSnapshot {
        self.inner.metrics.snapshot()
    }

    pub async fn open_tcp(
        &self,
        host: String,
        port: u16,
        egress_proxy: Option<String>,
    ) -> Result<(u32, mpsc::Receiver<Bytes>)> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (pending_tx, pending_rx) = oneshot::channel();
        let (data_tx, data_rx) = mpsc::channel(TCP_STREAM_CHANNEL_CAPACITY);
        self.inner.pending_tcp.lock().await.insert(id, pending_tx);
        self.inner.tcp_streams.write().await.insert(id, data_tx);
        if let Err(err) = self
            .inner
            .writer
            .send(Frame::OpenTcp {
                id,
                host,
                port,
                egress_proxy,
            })
            .await
        {
            self.inner.pending_tcp.lock().await.remove(&id);
            self.inner.tcp_streams.write().await.remove(&id);
            return Err(err).context("remote bridge is not available");
        }
        let result = match time::timeout(OPEN_TCP_TIMEOUT, pending_rx).await {
            Ok(result) => result.context("remote bridge closed before CONNECT completed")?,
            Err(_) => {
                self.inner.pending_tcp.lock().await.remove(&id);
                self.inner.tcp_streams.write().await.remove(&id);
                bail!(
                    "remote TCP open timed out after {}s",
                    OPEN_TCP_TIMEOUT.as_secs()
                );
            }
        };
        match result {
            Ok(()) => Ok((id, data_rx)),
            Err(message) => {
                self.inner.tcp_streams.write().await.remove(&id);
                self.inner.pending_tcp.lock().await.remove(&id);
                Err(anyhow!(message))
            }
        }
    }

    pub async fn send_data(&self, id: u32, data: Bytes) -> Result<()> {
        self.inner
            .writer
            .send(Frame::Data { id, data })
            .await
            .context("failed to send data to remote bridge")
    }

    pub async fn close(&self, id: u32, reason: impl Into<String>) {
        let _ = self
            .inner
            .writer
            .send(Frame::Close {
                id,
                reason: reason.into(),
            })
            .await;
        self.inner.tcp_streams.write().await.remove(&id);
    }

    pub async fn register_udp(&self) -> (u32, mpsc::Receiver<UdpDatagram>) {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(TCP_STREAM_CHANNEL_CAPACITY);
        self.inner.udp_streams.write().await.insert(id, tx);
        (id, rx)
    }

    pub async fn send_udp(&self, id: u32, datagram: UdpDatagram) -> Result<()> {
        self.inner
            .writer
            .send(Frame::UdpPacket {
                id,
                host: datagram.host,
                port: datagram.port,
                data: datagram.data,
            })
            .await
            .context("failed to send UDP packet to remote bridge")
    }

    pub async fn unregister_udp(&self, id: u32) {
        self.inner.udp_streams.write().await.remove(&id);
        let _ = self
            .inner
            .writer
            .send(Frame::Close {
                id,
                reason: "udp association closed".to_string(),
            })
            .await;
    }

    pub async fn send_log(&self, message: impl Into<String>) {
        let _ = self
            .inner
            .writer
            .send(Frame::Log {
                message: message.into(),
            })
            .await;
    }
}

impl Bridge {
    pub async fn connect_via_ssh(args: &cli::ProxyArgs) -> Result<Self> {
        let opened = deploy::open_remote_helper(args).await?;
        let mut bridge = connect_stream(opened.stream).await?;
        bridge.selected_protocol = Some(opened.protocol);
        bridge.transport_timings = opened.timings;
        Ok(bridge)
    }
}

pub async fn connect_stream<T>(stream: T) -> Result<Bridge>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (reader, writer) = io::split(stream);
    connect_io(reader, writer).await
}

pub async fn connect_io<R, W>(reader: R, mut writer: W) -> Result<Bridge>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut rx) = mpsc::channel::<Frame>(FRAME_CHANNEL_CAPACITY);
    let inner = Arc::new(BridgeInner {
        writer: tx,
        next_id: AtomicU32::new(1),
        pending_tcp: Mutex::new(HashMap::new()),
        tcp_streams: RwLock::new(HashMap::new()),
        udp_streams: RwLock::new(HashMap::new()),
        metrics: Arc::new(BridgeMetrics::default()),
    });

    let writer_metrics = inner.metrics.clone();
    let writer_task = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            match write_frame_batch(&mut writer, frame, &mut rx).await {
                Ok(stats) => writer_metrics.record_write_batch(&stats),
                Err(err) => {
                    writer_metrics.record_write_failure();
                    warn!(error = %err, "bridge writer stopped");
                    break;
                }
            }
        }
    });

    let reader_inner = inner.clone();
    let reader_task = tokio::spawn(async move {
        if let Err(err) = reader_loop(reader, reader_inner).await {
            warn!(error = %err, "bridge reader stopped");
        }
    });

    let lifecycle_inner = inner.clone();
    let lifecycle = tokio::spawn(async move {
        reader_task.await.ok();
        {
            let mut pending = lifecycle_inner.pending_tcp.lock().await;
            for (_, tx) in pending.drain() {
                let _ = tx.send(Err("remote bridge closed".to_string()));
            }
        }
        lifecycle_inner.tcp_streams.write().await.clear();
        lifecycle_inner.udp_streams.write().await.clear();
        writer_task.abort();
    });

    Ok(Bridge {
        handle: BridgeHandle { inner },
        lifecycle,
        selected_protocol: None,
        transport_timings: deploy::RemoteHelperTimings::default(),
    })
}

async fn reader_loop<R>(mut reader: R, inner: Arc<BridgeInner>) -> Result<()>
where
    R: AsyncRead + Unpin,
{
    let mut frame_reader = FrameReader::new();
    while let Some(frame) = frame_reader.read_from(&mut reader).await? {
        inner.metrics.record_read_frame(&frame);
        match frame {
            Frame::OpenTcpResult { id, ok, message } => {
                if let Some(tx) = inner.pending_tcp.lock().await.remove(&id) {
                    let _ = tx.send(if ok { Ok(()) } else { Err(message) });
                }
            }
            Frame::Data { id, data } => {
                let tx = inner.tcp_streams.read().await.get(&id).cloned();
                if let Some(tx) = tx {
                    match time::timeout(TCP_STREAM_BACKPRESSURE_TIMEOUT, tx.send(data)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) => {
                            inner.tcp_streams.write().await.remove(&id);
                        }
                        Err(_) => {
                            inner.metrics.record_tcp_backpressure_timeout();
                            warn!(
                                id,
                                timeout_secs = TCP_STREAM_BACKPRESSURE_TIMEOUT.as_secs(),
                                "local TCP stream receiver backpressure timed out"
                            );
                            inner.tcp_streams.write().await.remove(&id);
                            let _ = inner
                                .writer
                                .send(Frame::Close {
                                    id,
                                    reason: format!(
                                        "local receiver backpressure timed out after {}s",
                                        TCP_STREAM_BACKPRESSURE_TIMEOUT.as_secs()
                                    ),
                                })
                                .await;
                        }
                    }
                }
            }
            Frame::UdpPacket {
                id,
                host,
                port,
                data,
            } => {
                let tx = inner.udp_streams.read().await.get(&id).cloned();
                if let Some(tx) = tx {
                    match time::timeout(
                        UDP_ASSOC_BACKPRESSURE_TIMEOUT,
                        tx.send(UdpDatagram { host, port, data }),
                    )
                    .await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) => {
                            inner.udp_streams.write().await.remove(&id);
                        }
                        Err(_) => {
                            inner.metrics.record_udp_backpressure_timeout();
                            warn!(
                                id,
                                timeout_secs = UDP_ASSOC_BACKPRESSURE_TIMEOUT.as_secs(),
                                "local UDP association receiver backpressure timed out"
                            );
                            inner.udp_streams.write().await.remove(&id);
                            let _ = inner
                                .writer
                                .send(Frame::Close {
                                    id,
                                    reason: format!(
                                        "local UDP receiver backpressure timed out after {}s",
                                        UDP_ASSOC_BACKPRESSURE_TIMEOUT.as_secs()
                                    ),
                                })
                                .await;
                        }
                    }
                }
            }
            Frame::Close { id, reason } => {
                debug!(id, %reason, "remote closed stream");
                inner.tcp_streams.write().await.remove(&id);
                inner.udp_streams.write().await.remove(&id);
                if let Some(tx) = inner.pending_tcp.lock().await.remove(&id) {
                    let _ = tx.send(Err(reason));
                }
            }
            Frame::Log { message } => info!(target: "remote", %message),
            Frame::OpenTcp { .. } => warn!("controller received unexpected OpenTcp frame"),
        }
    }
    Ok(())
}
