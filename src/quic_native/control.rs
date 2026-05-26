#![allow(dead_code)]

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const CONTROL_FRAME_MAGIC: &[u8; 4] = b"QNC1";
pub const CONTROL_FRAME_VERSION: u16 = 1;
pub const MAX_CONTROL_FRAME: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteControlHello {
    pub version: u16,
    pub route_id: String,
    pub node: String,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub preferred_protocols: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteControlWelcome {
    pub version: u16,
    pub route_id: String,
    pub accepted: bool,
    pub selected_protocol: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RouteControlFrame {
    Hello(RouteControlHello),
    Welcome(RouteControlWelcome),
    Ping { seq: u64 },
    Pong { seq: u64 },
}

impl RouteControlFrame {
    pub async fn write_to<W>(&self, writer: &mut W) -> Result<()>
    where
        W: AsyncWrite + Unpin,
    {
        let payload =
            serde_json::to_vec(self).context("failed to encode QUIC-native control frame")?;
        if payload.len() > MAX_CONTROL_FRAME {
            bail!("QUIC-native control frame too large: {}", payload.len());
        }
        writer.write_all(CONTROL_FRAME_MAGIC).await?;
        writer
            .write_all(&CONTROL_FRAME_VERSION.to_be_bytes())
            .await?;
        writer
            .write_all(&(payload.len() as u32).to_be_bytes())
            .await?;
        writer.write_all(&payload).await?;
        writer.flush().await?;
        Ok(())
    }

    pub async fn read_from<R>(reader: &mut R) -> Result<Self>
    where
        R: AsyncRead + Unpin,
    {
        let mut magic = [0_u8; 4];
        reader
            .read_exact(&mut magic)
            .await
            .context("failed to read QUIC-native control frame magic")?;
        if &magic != CONTROL_FRAME_MAGIC {
            bail!("invalid QUIC-native control frame magic");
        }
        let mut version = [0_u8; 2];
        reader
            .read_exact(&mut version)
            .await
            .context("failed to read QUIC-native control frame version")?;
        let version = u16::from_be_bytes(version);
        if version != CONTROL_FRAME_VERSION {
            bail!(
                "unsupported QUIC-native control frame version {version}; expected {CONTROL_FRAME_VERSION}"
            );
        }
        let mut len = [0_u8; 4];
        reader
            .read_exact(&mut len)
            .await
            .context("failed to read QUIC-native control frame length")?;
        let len = u32::from_be_bytes(len) as usize;
        if len > MAX_CONTROL_FRAME {
            bail!("QUIC-native control frame too large: {len}");
        }
        let mut payload = vec![0_u8; len];
        reader
            .read_exact(&mut payload)
            .await
            .context("failed to read QUIC-native control frame payload")?;
        serde_json::from_slice(&payload).context("failed to decode QUIC-native control frame")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn control_frame_round_trips() {
        let frame = RouteControlFrame::Hello(RouteControlHello {
            version: CONTROL_FRAME_VERSION,
            route_id: "route-1".to_string(),
            node: "local".to_string(),
            features: vec!["ssh-native-direct-tcpip".to_string()],
            preferred_protocols: vec!["quic-native".to_string()],
        });
        let (mut client, mut server) = duplex(16 * 1024);

        frame.write_to(&mut client).await.unwrap();
        let decoded = RouteControlFrame::read_from(&mut server).await.unwrap();

        assert_eq!(decoded, frame);
    }
}
