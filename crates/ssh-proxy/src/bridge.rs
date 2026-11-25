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
        TCP_STREAM_CHANNEL_CAPACITY, UdpDatagram, write_frame_batch,
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
    // Dropping the route owner also wakes the I/O task for normal cleanup.
    _close_on_drop: oneshot::Sender<()>,
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
    pub tcp_queue_overflow_closes: u64,
    pub udp_queue_overflow_drops: u64,
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
        self.tcp_queue_overflow_closes += other.tcp_queue_overflow_closes;
        self.udp_queue_overflow_drops += other.udp_queue_overflow_drops;
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
    tcp_queue_overflow_closes: AtomicU64,
    udp_queue_overflow_drops: AtomicU64,
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

    fn record_tcp_queue_overflow(&self) {
        self.tcp_queue_overflow_closes
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_udp_queue_overflow(&self) {
        self.udp_queue_overflow_drops
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
            tcp_queue_overflow_closes: self.tcp_queue_overflow_closes.load(Ordering::Relaxed),
            udp_queue_overflow_drops: self.udp_queue_overflow_drops.load(Ordering::Relaxed),
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
    let write_frames = async move {
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
    };

    let reader_inner = inner.clone();
    let lifecycle_inner = inner.clone();
    let (close_on_drop, closed) = oneshot::channel();
    let lifecycle = tokio::spawn(async move {
        tokio::select! {
            _ = closed => {}
            _ = write_frames => {}
            result = reader_loop(reader, reader_inner) => {
                if let Err(err) = result {
                    warn!(error = %err, "bridge reader stopped");
                }
            }
        }
        {
            let mut pending = lifecycle_inner.pending_tcp.lock().await;
            for (_, tx) in pending.drain() {
                let _ = tx.send(Err("remote bridge closed".to_string()));
            }
        }
        lifecycle_inner.tcp_streams.write().await.clear();
        lifecycle_inner.udp_streams.write().await.clear();
    });

    Ok(Bridge {
        handle: BridgeHandle { inner },
        lifecycle,
        _close_on_drop: close_on_drop,
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
                    match tx.try_send(data) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            inner.tcp_streams.write().await.remove(&id);
                        }
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            inner.metrics.record_tcp_queue_overflow();
                            inner.tcp_streams.write().await.remove(&id);
                            // Losing a TCP frame invalidates this stream. If the writer cannot
                            // carry its close, reconnect to release the peer's stream as well.
                            inner
                                .writer
                                .try_send(Frame::Close {
                                    id,
                                    reason: "local TCP receiver queue full".into(),
                                })
                                .context("cannot signal overloaded TCP stream")?;
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
                    match tx.try_send(UdpDatagram { host, port, data }) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            inner.metrics.record_udp_queue_overflow();
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            inner.udp_streams.write().await.remove(&id);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writer_failure_ends_bridge_with_idle_reader() {
        let (reader, _idle_peer) = io::duplex(64);
        let (writer, disconnected) = io::duplex(64);
        drop(disconnected);
        let bridge = connect_io(reader, writer).await.unwrap();
        let open = bridge.handle.open_tcp("localhost".into(), 80, None);
        let (result, ended) = time::timeout(Duration::from_secs(2), async {
            tokio::join!(open, bridge.lifecycle)
        })
        .await
        .unwrap();
        assert!(result.is_err());
        ended.unwrap();
    }

    #[tokio::test]
    async fn full_tcp_receiver_does_not_delay_another_stream() {
        let (reader, mut peer) = io::duplex(4096);
        let bridge = connect_io(reader, io::sink()).await.unwrap();
        let (slow, _stalled) = mpsc::channel(1);
        slow.try_send(Bytes::from_static(b"full")).unwrap();
        let (fast, mut received) = mpsc::channel(1);
        {
            let mut streams = bridge.handle.inner.tcp_streams.write().await;
            streams.insert(1, slow);
            streams.insert(2, fast);
        }
        let (_tx, mut empty) = mpsc::channel(1);
        for id in [1, 2] {
            write_frame_batch(
                &mut peer,
                Frame::Data {
                    id,
                    data: Bytes::from_static(b"x"),
                },
                &mut empty,
            )
            .await
            .unwrap();
        }
        assert_eq!(
            time::timeout(Duration::from_secs(1), received.recv())
                .await
                .unwrap()
                .unwrap(),
            &b"x"[..]
        );
        drop(peer);
        bridge.lifecycle.await.unwrap();
    }
}
