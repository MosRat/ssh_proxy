use std::{
    io::{Error, ErrorKind, IoSlice},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use bytes::{Bytes, BytesMut};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::mpsc,
};

pub const MAX_FRAME: usize = 16 * 1024 * 1024;
pub const TCP_DATA_CHUNK: usize = 128 * 1024;
pub const FRAME_CHANNEL_CAPACITY: usize = 256;
pub const TCP_STREAM_CHANNEL_CAPACITY: usize = 64;
pub const FRAME_WRITE_BATCH_LIMIT: usize = 32;
pub const TCP_STREAM_BACKPRESSURE_TIMEOUT: Duration = Duration::from_secs(30);
pub const UDP_ASSOC_BACKPRESSURE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameWriteBatchStats {
    pub frames_written: usize,
    pub flushes: usize,
    pub data_frames_written: usize,
    pub data_bytes_written: usize,
    pub vectored_writes: usize,
}

impl FrameWriteBatchStats {
    fn record_frame(&mut self, frame: &Frame) {
        self.frames_written += 1;
        if let Frame::Data { data, .. } = frame {
            self.data_frames_written += 1;
            self.data_bytes_written += data.len();
        }
    }
}

#[derive(Debug, Default)]
struct FrameWriteScratch {
    header: [u8; 9],
    payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct UdpDatagram {
    pub host: String,
    pub port: u16,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum Frame {
    OpenTcp {
        id: u32,
        host: String,
        port: u16,
        egress_proxy: Option<String>,
    },
    OpenTcpResult {
        id: u32,
        ok: bool,
        message: String,
    },
    Data {
        id: u32,
        data: Bytes,
    },
    Close {
        id: u32,
        reason: String,
    },
    UdpPacket {
        id: u32,
        host: String,
        port: u16,
        data: Vec<u8>,
    },
    Log {
        message: String,
    },
}

impl Frame {
    #[cfg(test)]
    pub async fn read_from<R>(reader: &mut R) -> Result<Option<Self>>
    where
        R: AsyncRead + Unpin,
    {
        FrameReader::new().read_from(reader).await
    }

    #[cfg(test)]
    async fn write_to<W>(&self, writer: &mut W) -> Result<()>
    where
        W: AsyncWrite + Unpin,
    {
        let mut scratch = FrameWriteScratch::default();
        self.write_to_with_scratch(writer, &mut scratch).await?;
        Ok(())
    }

    async fn write_to_with_scratch<W>(
        &self,
        writer: &mut W,
        scratch: &mut FrameWriteScratch,
    ) -> Result<usize>
    where
        W: AsyncWrite + Unpin,
    {
        match self {
            Frame::Data { id, data } => {
                write_payload(writer, &mut scratch.header, 3, *id, data).await
            }
            _ => {
                let (ty, id) = self.encode_into(&mut scratch.payload)?;
                write_payload(writer, &mut scratch.header, ty, id, &scratch.payload).await
            }
        }
    }

    fn encode_into(&self, payload: &mut Vec<u8>) -> Result<(u8, u32)> {
        payload.clear();
        let capacity = self.encoded_payload_capacity();
        if payload.capacity() < capacity {
            payload.reserve(capacity - payload.capacity());
        }
        let (ty, id) = match self {
            Frame::OpenTcp {
                id,
                host,
                port,
                egress_proxy,
            } => {
                write_string(payload, host)?;
                payload.extend_from_slice(&port.to_be_bytes());
                match egress_proxy {
                    Some(proxy) => {
                        payload.push(1);
                        write_string(payload, proxy)?;
                    }
                    None => payload.push(0),
                }
                (1, *id)
            }
            Frame::OpenTcpResult { id, ok, message } => {
                payload.push(u8::from(*ok));
                write_string(payload, message)?;
                (2, *id)
            }
            Frame::Data { id, data } => {
                payload.extend_from_slice(data);
                (3, *id)
            }
            Frame::Close { id, reason } => {
                write_string(payload, reason)?;
                (4, *id)
            }
            Frame::UdpPacket {
                id,
                host,
                port,
                data,
            } => {
                write_string(payload, host)?;
                payload.extend_from_slice(&port.to_be_bytes());
                payload.extend_from_slice(&(data.len() as u32).to_be_bytes());
                payload.extend_from_slice(data);
                (5, *id)
            }
            Frame::Log { message } => {
                write_string(payload, message)?;
                (6, 0)
            }
        };
        Ok((ty, id))
    }

    fn encoded_payload_capacity(&self) -> usize {
        match self {
            Frame::OpenTcp {
                host, egress_proxy, ..
            } => {
                2 + host.len()
                    + 2
                    + 1
                    + egress_proxy
                        .as_ref()
                        .map(|proxy| 2 + proxy.len())
                        .unwrap_or(0)
            }
            Frame::OpenTcpResult { message, .. } => 1 + 2 + message.len(),
            Frame::Data { data, .. } => data.len(),
            Frame::Close { reason, .. } => 2 + reason.len(),
            Frame::UdpPacket { host, data, .. } => 2 + host.len() + 2 + 4 + data.len(),
            Frame::Log { message } => 2 + message.len(),
        }
    }

    fn decode(ty: u8, id: u32, payload: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(payload);
        let frame = match ty {
            1 => {
                let host = cursor.read_string()?;
                let port = cursor.read_u16()?;
                let egress_proxy = if cursor.is_empty() {
                    None
                } else if cursor.read_u8()? == 0 {
                    None
                } else {
                    Some(cursor.read_string()?)
                };
                cursor.ensure_empty()?;
                Frame::OpenTcp {
                    id,
                    host,
                    port,
                    egress_proxy,
                }
            }
            2 => {
                let ok = cursor.read_u8()? != 0;
                let message = cursor.read_string()?;
                cursor.ensure_empty()?;
                Frame::OpenTcpResult { id, ok, message }
            }
            3 => Frame::Data {
                id,
                data: Bytes::copy_from_slice(payload),
            },
            4 => {
                let reason = cursor.read_string()?;
                cursor.ensure_empty()?;
                Frame::Close { id, reason }
            }
            5 => {
                let host = cursor.read_string()?;
                let port = cursor.read_u16()?;
                let len = cursor.read_u32()? as usize;
                let data = cursor.read_bytes(len)?.to_vec();
                cursor.ensure_empty()?;
                Frame::UdpPacket {
                    id,
                    host,
                    port,
                    data,
                }
            }
            6 => {
                let message = cursor.read_string()?;
                cursor.ensure_empty()?;
                Frame::Log { message }
            }
            _ => bail!("unknown frame type {ty}"),
        };
        Ok(frame)
    }
}

pub struct FrameReader {
    header: [u8; 9],
    structured_payload: Vec<u8>,
    data_payload: BytesMut,
}

impl FrameReader {
    pub fn new() -> Self {
        Self {
            header: [0; 9],
            structured_payload: Vec::new(),
            data_payload: BytesMut::new(),
        }
    }

    pub async fn read_from<R>(&mut self, reader: &mut R) -> Result<Option<Frame>>
    where
        R: AsyncRead + Unpin,
    {
        match reader.read_exact(&mut self.header).await {
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(err) => return Err(err).context("failed to read frame header"),
        }
        let ty = self.header[0];
        let id = u32::from_be_bytes(self.header[1..5].try_into()?);
        let len = u32::from_be_bytes(self.header[5..9].try_into()?) as usize;
        if len > MAX_FRAME {
            bail!("frame too large: {len}");
        }
        if ty == 3 {
            return self.read_data_frame(reader, id, len).await.map(Some);
        }
        self.structured_payload.resize(len, 0);
        reader
            .read_exact(&mut self.structured_payload)
            .await
            .context("failed to read frame payload")?;
        Frame::decode(ty, id, &self.structured_payload).map(Some)
    }

    async fn read_data_frame<R>(&mut self, reader: &mut R, id: u32, len: usize) -> Result<Frame>
    where
        R: AsyncRead + Unpin,
    {
        self.data_payload.clear();
        self.data_payload.reserve(len);
        while self.data_payload.len() < len {
            let remaining = len - self.data_payload.len();
            let mut limited = reader.take(remaining as u64);
            let read = limited
                .read_buf(&mut self.data_payload)
                .await
                .context("failed to read frame payload")?;
            if read == 0 {
                bail!("failed to read frame payload: early eof");
            }
        }
        Ok(Frame::Data {
            id,
            data: self.data_payload.split_to(len).freeze(),
        })
    }
}

impl Default for FrameReader {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn write_frame_batch<W>(
    writer: &mut W,
    first: Frame,
    rx: &mut mpsc::Receiver<Frame>,
) -> Result<FrameWriteBatchStats>
where
    W: AsyncWrite + Unpin,
{
    let mut scratch = FrameWriteScratch::default();
    let mut stats = FrameWriteBatchStats::default();
    stats.vectored_writes += first.write_to_with_scratch(writer, &mut scratch).await?;
    stats.record_frame(&first);
    while stats.frames_written < FRAME_WRITE_BATCH_LIMIT {
        match rx.try_recv() {
            Ok(frame) => {
                stats.vectored_writes += frame.write_to_with_scratch(writer, &mut scratch).await?;
                stats.record_frame(&frame);
            }
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {
                break;
            }
        }
    }
    writer.flush().await?;
    stats.flushes = 1;
    Ok(stats)
}

async fn write_payload<W, B>(
    writer: &mut W,
    header: &mut [u8; 9],
    ty: u8,
    id: u32,
    payload: B,
) -> Result<usize>
where
    W: AsyncWrite + Unpin,
    B: AsRef<[u8]>,
{
    let payload = payload.as_ref();
    if payload.len() > MAX_FRAME {
        bail!("frame too large: {}", payload.len());
    }
    header[0] = ty;
    header[1..5].copy_from_slice(&id.to_be_bytes());
    header[5..9].copy_from_slice(&(payload.len() as u32).to_be_bytes());
    write_all_vectored_payload(writer, header, payload).await
}

async fn write_all_vectored_payload<W>(
    writer: &mut W,
    header: &[u8],
    payload: &[u8],
) -> Result<usize>
where
    W: AsyncWrite + Unpin,
{
    let mut header_offset = 0;
    let mut payload_offset = 0;
    let mut writes = 0;
    while header_offset < header.len() || payload_offset < payload.len() {
        let written = if header_offset < header.len() && payload_offset < payload.len() {
            writer
                .write_vectored(&[
                    IoSlice::new(&header[header_offset..]),
                    IoSlice::new(&payload[payload_offset..]),
                ])
                .await?
        } else if header_offset < header.len() {
            writer
                .write_vectored(&[IoSlice::new(&header[header_offset..])])
                .await?
        } else {
            writer
                .write_vectored(&[IoSlice::new(&payload[payload_offset..])])
                .await?
        };
        if written == 0 {
            return Err(Error::new(ErrorKind::WriteZero, "failed to write frame").into());
        }
        writes += 1;
        let header_remaining = header.len() - header_offset;
        if written < header_remaining {
            header_offset += written;
        } else {
            header_offset = header.len();
            payload_offset += written - header_remaining;
        }
    }
    Ok(writes)
}

fn write_string(out: &mut Vec<u8>, value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    if bytes.len() > u16::MAX as usize {
        bail!("string too long");
    }
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn read_u8(&mut self) -> Result<u8> {
        let value = *self
            .bytes
            .get(self.pos)
            .ok_or_else(|| anyhow!("payload truncated"))?;
        self.pos += 1;
        Ok(value)
    }

    fn read_u16(&mut self) -> Result<u16> {
        let bytes = self.read_bytes(2)?;
        Ok(u16::from_be_bytes(bytes.try_into()?))
    }

    fn read_u32(&mut self) -> Result<u32> {
        let bytes = self.read_bytes(4)?;
        Ok(u32::from_be_bytes(bytes.try_into()?))
    }

    fn read_string(&mut self) -> Result<String> {
        let len = self.read_u16()? as usize;
        let bytes = self.read_bytes(len)?;
        String::from_utf8(bytes.to_vec()).context("invalid utf-8 string in frame")
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| anyhow!("payload length overflow"))?;
        let bytes = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| anyhow!("payload truncated"))?;
        self.pos = end;
        Ok(bytes)
    }

    fn ensure_empty(&self) -> Result<()> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            bail!(
                "frame payload has {} trailing bytes",
                self.bytes.len() - self.pos
            )
        }
    }

    fn is_empty(&self) -> bool {
        self.pos == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        pin::Pin,
        task::{Context as TaskContext, Poll},
    };
    use tokio::io::AsyncWrite;

    struct ChunkedVectoredWriter {
        out: Vec<u8>,
        max_per_call: usize,
        vectored_calls: usize,
        multi_slice_calls: usize,
    }

    impl ChunkedVectoredWriter {
        fn new(max_per_call: usize) -> Self {
            Self {
                out: Vec::new(),
                max_per_call,
                vectored_calls: 0,
                multi_slice_calls: 0,
            }
        }
    }

    impl AsyncWrite for ChunkedVectoredWriter {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut TaskContext<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            let len = buf.len().min(self.max_per_call);
            self.out.extend_from_slice(&buf[..len]);
            Poll::Ready(Ok(len))
        }

        fn poll_write_vectored(
            mut self: Pin<&mut Self>,
            _cx: &mut TaskContext<'_>,
            bufs: &[IoSlice<'_>],
        ) -> Poll<std::io::Result<usize>> {
            self.vectored_calls += 1;
            if bufs.len() > 1 && !bufs[1].is_empty() {
                self.multi_slice_calls += 1;
            }
            let mut remaining = self.max_per_call;
            let mut written = 0;
            for buf in bufs {
                if remaining == 0 {
                    break;
                }
                let len = buf.len().min(remaining);
                self.out.extend_from_slice(&buf[..len]);
                remaining -= len;
                written += len;
            }
            Poll::Ready(Ok(written))
        }

        fn is_write_vectored(&self) -> bool {
            true
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _cx: &mut TaskContext<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: Pin<&mut Self>,
            _cx: &mut TaskContext<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[test]
    fn structured_frames_reject_trailing_payload() {
        let mut payload = Vec::new();
        write_string(&mut payload, "example.com").unwrap();
        payload.extend_from_slice(&443_u16.to_be_bytes());
        payload.push(0);
        payload.extend_from_slice(b"junk");

        let err = Frame::decode(1, 7, &payload).unwrap_err().to_string();

        assert!(err.contains("trailing bytes"), "{err}");
    }

    #[test]
    fn structured_encode_reuses_scratch_capacity() {
        let mut payload = Vec::with_capacity(64);
        let frame = Frame::OpenTcp {
            id: 7,
            host: "example.com".to_string(),
            port: 443,
            egress_proxy: Some("socks5h://127.0.0.1:1080".to_string()),
        };

        frame.encode_into(&mut payload).unwrap();
        let capacity = payload.capacity();
        let len = payload.len();

        Frame::Close {
            id: 7,
            reason: "done".to_string(),
        }
        .encode_into(&mut payload)
        .unwrap();

        assert!(capacity >= len);
        assert_eq!(payload.capacity(), capacity);
    }

    #[tokio::test]
    async fn write_rejects_oversized_data_frame() {
        let frame = Frame::Data {
            id: 1,
            data: Bytes::from(vec![0_u8; MAX_FRAME + 1]),
        };
        let mut out = Vec::new();

        let err = frame.write_to(&mut out).await.unwrap_err().to_string();

        assert!(err.contains("frame too large"), "{err}");
    }

    #[tokio::test]
    async fn write_frame_batch_drains_ready_frames() {
        let (tx, mut rx) = mpsc::channel(4);
        tx.send(Frame::Close {
            id: 2,
            reason: "done".to_string(),
        })
        .await
        .unwrap();
        tx.send(Frame::Data {
            id: 3,
            data: Bytes::from_static(b"abc"),
        })
        .await
        .unwrap();

        let mut out = Vec::new();
        let stats = write_frame_batch(
            &mut out,
            Frame::OpenTcpResult {
                id: 1,
                ok: true,
                message: String::new(),
            },
            &mut rx,
        )
        .await
        .unwrap();

        assert_eq!(stats.frames_written, 3);
        assert_eq!(stats.flushes, 1);
        assert_eq!(stats.data_frames_written, 1);
        assert_eq!(stats.data_bytes_written, 3);
        assert_eq!(stats.vectored_writes, 3);
        let mut cursor = std::io::Cursor::new(out);
        assert!(matches!(
            Frame::read_from(&mut cursor).await.unwrap(),
            Some(Frame::OpenTcpResult {
                id: 1,
                ok: true,
                ..
            })
        ));
        assert!(matches!(
            Frame::read_from(&mut cursor).await.unwrap(),
            Some(Frame::Close { id: 2, .. })
        ));
        assert!(matches!(
            Frame::read_from(&mut cursor).await.unwrap(),
            Some(Frame::Data { id: 3, .. })
        ));
    }

    #[tokio::test]
    async fn data_frame_write_uses_vectored_header_and_payload() {
        let (tx, mut rx) = mpsc::channel(1);
        drop(tx);
        let mut out = ChunkedVectoredWriter::new(5);

        let stats = write_frame_batch(
            &mut out,
            Frame::Data {
                id: 9,
                data: Bytes::from_static(b"abcdef"),
            },
            &mut rx,
        )
        .await
        .unwrap();

        assert!(out.vectored_calls > 1);
        assert!(out.multi_slice_calls >= 1);
        assert_eq!(stats.frames_written, 1);
        assert_eq!(stats.vectored_writes, out.vectored_calls);
        let mut cursor = std::io::Cursor::new(out.out);
        assert!(matches!(
            Frame::read_from(&mut cursor).await.unwrap(),
            Some(Frame::Data { id: 9, data }) if data.as_ref() == b"abcdef"
        ));
    }
}
