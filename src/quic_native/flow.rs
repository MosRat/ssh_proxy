#![allow(dead_code)]

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::stream_header::{MAX_STREAM_HEADER, StreamHeader};

const STREAM_HEADER_PREFIX_LEN: usize = 8;

pub async fn write_flow_header<W>(writer: &mut W, header: &StreamHeader) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut encoded = Vec::with_capacity(header.encoded_capacity_hint());
    write_flow_header_with_buffer(writer, header, &mut encoded).await
}

pub async fn write_flow_header_with_buffer<W>(
    writer: &mut W,
    header: &StreamHeader,
    encoded: &mut Vec<u8>,
) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    header.encode_into(encoded)?;
    writer
        .write_all(encoded)
        .await
        .context("failed to write QUIC-native flow header")?;
    writer
        .flush()
        .await
        .context("failed to flush QUIC-native flow header")?;
    Ok(())
}

pub async fn read_flow_header<R>(reader: &mut R) -> Result<StreamHeader>
where
    R: AsyncRead + Unpin,
{
    let mut prefix = [0_u8; STREAM_HEADER_PREFIX_LEN];
    reader
        .read_exact(&mut prefix)
        .await
        .context("failed to read QUIC-native flow header prefix")?;
    let body_len = u16::from_be_bytes(prefix[6..8].try_into()?) as usize;
    if body_len > MAX_STREAM_HEADER {
        bail!("QUIC-native flow header too large: {body_len}");
    }
    let mut encoded = Vec::with_capacity(STREAM_HEADER_PREFIX_LEN + body_len);
    encoded.extend_from_slice(&prefix);
    encoded.resize(STREAM_HEADER_PREFIX_LEN + body_len, 0);
    reader
        .read_exact(&mut encoded[STREAM_HEADER_PREFIX_LEN..])
        .await
        .context("failed to read QUIC-native flow header body")?;
    let (header, consumed) = StreamHeader::decode(&encoded)?;
    if consumed != encoded.len() {
        bail!(
            "QUIC-native flow header decoder consumed {consumed} bytes from {} bytes",
            encoded.len()
        );
    }
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quic_native::stream_header::{StreamTarget, TargetKind};
    use tokio::io::duplex;

    #[tokio::test]
    async fn flow_header_round_trips_over_async_io() {
        let header = StreamHeader {
            route_id: "route-1".to_string(),
            stream_id: 7,
            target: StreamTarget {
                kind: TargetKind::TcpConnect,
                host: "example.com".to_string(),
                port: 443,
            },
            egress_proxy: None,
            flags: 0,
        };
        let (mut client, mut server) = duplex(1024);

        write_flow_header(&mut client, &header).await.unwrap();
        let decoded = read_flow_header(&mut server).await.unwrap();

        assert_eq!(decoded, header);
    }
}
