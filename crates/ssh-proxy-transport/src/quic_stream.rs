use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub struct QuicBiStream {
    send: quinn::SendStream,
    recv: quinn::RecvStream,
    _connection: Option<quinn::Connection>,
    _endpoint: Option<quinn::Endpoint>,
    first_byte_recorded: Arc<AtomicBool>,
}

impl QuicBiStream {
    pub fn new(send: quinn::SendStream, recv: quinn::RecvStream) -> Self {
        Self {
            send,
            recv,
            _connection: None,
            _endpoint: None,
            first_byte_recorded: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_lifetime(
        send: quinn::SendStream,
        recv: quinn::RecvStream,
        connection: quinn::Connection,
        endpoint: quinn::Endpoint,
    ) -> Self {
        Self {
            send,
            recv,
            _connection: Some(connection),
            _endpoint: Some(endpoint),
            first_byte_recorded: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_connection(
        send: quinn::SendStream,
        recv: quinn::RecvStream,
        connection: quinn::Connection,
    ) -> Self {
        Self {
            send,
            recv,
            _connection: Some(connection),
            _endpoint: None,
            first_byte_recorded: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn reset(&mut self, error_code: quinn::VarInt) {
        let _ = self.send.reset(error_code);
        let _ = self.recv.stop(error_code);
    }

    pub fn reset_u32(&mut self, error_code: u32) {
        self.reset(quinn::VarInt::from_u32(error_code));
    }

    pub fn finish(&mut self) {
        let _ = self.send.finish();
    }

    pub fn first_byte_recorded(&self) -> Arc<AtomicBool> {
        self.first_byte_recorded.clone()
    }
}

impl AsyncRead for QuicBiStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.recv).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &poll {
            let after = buf.filled().len();
            if after > before {
                self.first_byte_recorded.store(true, Ordering::Relaxed);
            }
        }
        poll
    }
}

impl AsyncWrite for QuicBiStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let poll = Pin::new(&mut self.send)
            .poll_write(cx, buf)
            .map_err(quic_write_error);
        if let Poll::Ready(Ok(written)) = poll {
            if written > 0 {
                self.first_byte_recorded.store(true, Ordering::Relaxed);
            }
        }
        poll
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.send).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.send).poll_shutdown(cx)
    }
}

fn quic_write_error(err: quinn::WriteError) -> std::io::Error {
    std::io::Error::other(err)
}
