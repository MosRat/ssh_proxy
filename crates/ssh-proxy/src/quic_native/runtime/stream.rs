use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context as TaskContext, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite};
use tracing::trace;

use super::Stream;

impl Drop for Stream {
    fn drop(&mut self) {
        if !self.closed.swap(true, Ordering::Relaxed) {
            self.inner
                .reset(quinn::VarInt::from_u32(super::super::FLOW_RESET_ERROR_CODE));
            self.state.record_quic_flow_drop();
            self.worker.record_flow_closed(true);
        }
        self.state
            .active_quic_flows
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_sub(1)
            })
            .ok();
    }
}

impl Stream {
    pub fn first_byte_recorded(&self) -> Arc<AtomicBool> {
        self.first_byte_recorded.clone()
    }

    pub fn record_backpressure_timeout(&self) {
        self.worker.record_backpressure_timeout();
    }

    pub async fn finish(mut self, reason: impl Into<String>) {
        if !self.closed.swap(true, Ordering::Relaxed) {
            self.inner.finish();
            self.worker.record_flow_closed(false);
            self.state.record_quic_flow_close(reason, false).await;
        }
    }

    pub async fn reset(&mut self, reason: impl Into<String>) {
        if !self.closed.swap(true, Ordering::Relaxed) {
            self.inner
                .reset(quinn::VarInt::from_u32(super::super::FLOW_RESET_ERROR_CODE));
            self.worker.record_flow_closed(true);
            self.state.record_quic_flow_close(reason, true).await;
        }
    }
}

impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &poll {
            let after = buf.filled().len();
            if after > before {
                trace!(
                    worker_id = self.worker.id,
                    bytes = after - before,
                    "received QUIC-native stream bytes"
                );
                self.worker
                    .bytes_remote_to_client
                    .fetch_add((after - before) as u64, Ordering::Relaxed);
            }
            if after > before && !self.first_byte_recorded.swap(true, Ordering::Relaxed) {
                self.state
                    .record_quic_flow_first_byte_latency(self.opened_at.elapsed());
            }
        }
        poll
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match Pin::new(&mut self.inner).poll_write(cx, buf) {
            Poll::Ready(Ok(written)) => {
                trace!(
                    worker_id = self.worker.id,
                    bytes = written,
                    "sent QUIC-native stream bytes"
                );
                self.worker
                    .bytes_client_to_remote
                    .fetch_add(written as u64, Ordering::Relaxed);
                Poll::Ready(Ok(written))
            }
            other => other,
        }
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
