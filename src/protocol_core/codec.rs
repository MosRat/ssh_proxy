use anyhow::Result;
use bytes::Bytes;
use tokio::{
    io::{AsyncRead, AsyncWrite},
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
}
