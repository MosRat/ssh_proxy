#![allow(dead_code)]

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::protocol_core::codec::{read_json_control_frame, write_json_control_frame};

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
        write_json_control_frame(
            writer,
            CONTROL_FRAME_MAGIC,
            CONTROL_FRAME_VERSION,
            MAX_CONTROL_FRAME,
            self,
            "QUIC-native control frame",
        )
        .await
    }

    pub async fn read_from<R>(reader: &mut R) -> Result<Self>
    where
        R: AsyncRead + Unpin,
    {
        read_json_control_frame(
            reader,
            CONTROL_FRAME_MAGIC,
            CONTROL_FRAME_VERSION,
            MAX_CONTROL_FRAME,
            "QUIC-native control frame",
        )
        .await
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
