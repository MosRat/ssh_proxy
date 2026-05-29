use anyhow::{Context, Result, bail};
use bytes::Bytes;
use serde::{Serialize, de::DeserializeOwned};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::mpsc,
};

use crate::protocol::{Frame, FrameReader, FrameWriteBatchStats, MAX_FRAME, write_frame_batch};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DataFrame {
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

impl From<Frame> for DataFrame {
    fn from(frame: Frame) -> Self {
        match frame {
            Frame::OpenTcp {
                id,
                host,
                port,
                egress_proxy,
            } => Self::OpenTcp {
                id,
                host,
                port,
                egress_proxy,
            },
            Frame::OpenTcpResult { id, ok, message } => Self::OpenTcpResult { id, ok, message },
            Frame::Data { id, data } => Self::Data { id, data },
            Frame::Close { id, reason } => Self::Close { id, reason },
            Frame::UdpPacket {
                id,
                host,
                port,
                data,
            } => Self::UdpPacket {
                id,
                host,
                port,
                data,
            },
            Frame::Log { message } => Self::Log { message },
        }
    }
}

impl From<DataFrame> for Frame {
    fn from(frame: DataFrame) -> Self {
        match frame {
            DataFrame::OpenTcp {
                id,
                host,
                port,
                egress_proxy,
            } => Self::OpenTcp {
                id,
                host,
                port,
                egress_proxy,
            },
            DataFrame::OpenTcpResult { id, ok, message } => Self::OpenTcpResult { id, ok, message },
            DataFrame::Data { id, data } => Self::Data { id, data },
            DataFrame::Close { id, reason } => Self::Close { id, reason },
            DataFrame::UdpPacket {
                id,
                host,
                port,
                data,
            } => Self::UdpPacket {
                id,
                host,
                port,
                data,
            },
            DataFrame::Log { message } => Self::Log { message },
        }
    }
}

#[derive(Default)]
pub(crate) struct SpxFrameCodec {
    reader: FrameReader,
}

impl SpxFrameCodec {
    pub(crate) fn new() -> Self {
        Self {
            reader: FrameReader::new(),
        }
    }

    pub(crate) async fn read_from<R>(&mut self, reader: &mut R) -> Result<Option<DataFrame>>
    where
        R: AsyncRead + Unpin,
    {
        self.reader
            .read_from(reader)
            .await
            .map(|frame| frame.map(DataFrame::from))
    }

    pub(crate) async fn write_to<W>(
        writer: &mut W,
        frame: DataFrame,
    ) -> Result<FrameWriteBatchStats>
    where
        W: AsyncWrite + Unpin,
    {
        let (tx, mut rx) = mpsc::channel(1);
        drop(tx);
        write_frame_batch(writer, frame.into(), &mut rx).await
    }
}

pub(crate) fn max_data_frame_len() -> usize {
    MAX_FRAME
}

pub(crate) async fn write_json_control_frame<W, T>(
    writer: &mut W,
    magic: &[u8; 4],
    version: u16,
    max_len: usize,
    value: &T,
    label: &'static str,
) -> Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let payload = serde_json::to_vec(value).with_context(|| format!("failed to encode {label}"))?;
    if payload.len() > max_len {
        bail!("{label} too large: {}", payload.len());
    }
    writer.write_all(magic).await?;
    writer.write_all(&version.to_be_bytes()).await?;
    writer
        .write_all(&(payload.len() as u32).to_be_bytes())
        .await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

pub(crate) async fn read_json_control_frame<R, T>(
    reader: &mut R,
    magic: &[u8; 4],
    expected_version: u16,
    max_len: usize,
    label: &'static str,
) -> Result<T>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut actual_magic = [0_u8; 4];
    reader
        .read_exact(&mut actual_magic)
        .await
        .with_context(|| format!("failed to read {label} magic"))?;
    if &actual_magic != magic {
        bail!("invalid {label} magic");
    }
    let mut version = [0_u8; 2];
    reader
        .read_exact(&mut version)
        .await
        .with_context(|| format!("failed to read {label} version"))?;
    let version = u16::from_be_bytes(version);
    if version != expected_version {
        bail!("unsupported {label} version {version}; expected {expected_version}");
    }
    let mut len = [0_u8; 4];
    reader
        .read_exact(&mut len)
        .await
        .with_context(|| format!("failed to read {label} length"))?;
    let len = u32::from_be_bytes(len) as usize;
    if len > max_len {
        bail!("{label} too large: {len}");
    }
    let mut payload = vec![0_u8; len];
    reader
        .read_exact(&mut payload)
        .await
        .with_context(|| format!("failed to read {label} payload"))?;
    serde_json::from_slice(&payload).with_context(|| format!("failed to decode {label}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spx_codec_preserves_current_data_header_bytes() {
        let mut out = Vec::new();

        SpxFrameCodec::write_to(
            &mut out,
            DataFrame::Data {
                id: 0x0102_0304,
                data: Bytes::from_static(b"abc"),
            },
        )
        .await
        .unwrap();

        assert_eq!(&out[..9], &[3, 1, 2, 3, 4, 0, 0, 0, 3]);
        assert_eq!(&out[9..], b"abc");

        let mut cursor = std::io::Cursor::new(out);
        let decoded = SpxFrameCodec::new().read_from(&mut cursor).await.unwrap();
        assert_eq!(
            decoded,
            Some(DataFrame::Data {
                id: 0x0102_0304,
                data: Bytes::from_static(b"abc")
            })
        );
    }

    #[tokio::test]
    async fn spx_codec_rejects_oversize_frame() {
        let mut out = Vec::new();

        let err = SpxFrameCodec::write_to(
            &mut out,
            DataFrame::Data {
                id: 1,
                data: Bytes::from(vec![0_u8; max_data_frame_len() + 1]),
            },
        )
        .await
        .unwrap_err()
        .to_string();

        assert!(err.contains("frame too large"), "{err}");
    }

    #[tokio::test]
    async fn json_control_frame_keeps_magic_version_length_outer_frame() {
        let mut out = Vec::new();
        let value = serde_json::json!({
            "kind": "ping",
            "seq": 7
        });

        write_json_control_frame(&mut out, b"TST1", 2, 1024, &value, "test control frame")
            .await
            .unwrap();

        assert_eq!(&out[..4], b"TST1");
        assert_eq!(&out[4..6], &2_u16.to_be_bytes());
        assert_eq!(u32::from_be_bytes(out[6..10].try_into().unwrap()), 23);

        let mut cursor = std::io::Cursor::new(out);
        let decoded: serde_json::Value =
            read_json_control_frame(&mut cursor, b"TST1", 2, 1024, "test control frame")
                .await
                .unwrap();
        assert_eq!(decoded["kind"], "ping");
        assert_eq!(decoded["seq"], 7);
    }
}
