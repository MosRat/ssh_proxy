use std::{fmt, net::SocketAddr, str::FromStr};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::version::PEER_PROTOCOL_VERSION;

pub const PEER_VERSION: u16 = PEER_PROTOCOL_VERSION;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PeerProtocol {
    SshNative,
    QuicNative,
    Quic,
    TlsTcp,
    Tcp,
    SshDirect,
    SshExec,
}

impl fmt::Display for PeerProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::SshNative => "ssh-native",
            Self::QuicNative => "quic-native",
            Self::Quic => "quic",
            Self::TlsTcp => "tls-tcp",
            Self::Tcp => "tcp",
            Self::SshDirect => "ssh-direct",
            Self::SshExec => "ssh-exec",
        };
        f.write_str(value)
    }
}

impl PeerProtocol {
    pub fn data_plane_label(self) -> &'static str {
        match self {
            Self::SshNative => "ssh-native",
            Self::QuicNative => "quic-native",
            Self::Quic => "quic-framed",
            Self::TlsTcp => "tls-spx-framed",
            Self::Tcp => "plain-spx-framed",
            Self::SshDirect => "ssh-direct-spx",
            Self::SshExec => "ssh-exec-spx",
        }
    }
}

impl FromStr for PeerProtocol {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "quic" => Ok(Self::Quic),
            "quic-native" | "quic_native" | "native-quic" | "native_quic" => Ok(Self::QuicNative),
            "ssh-native" | "ssh_native" | "native-ssh" | "native_ssh" => Ok(Self::SshNative),
            "tls-tcp" | "tls_tcp" | "tls" => Ok(Self::TlsTcp),
            "tcp" | "plain-tcp" | "plain_tcp" | "direct-tcp" | "direct_tcp" => Ok(Self::Tcp),
            "ssh-direct" | "ssh_direct" | "ssh-tcp" | "ssh_tcp" => Ok(Self::SshDirect),
            "ssh-exec" | "ssh_exec" | "exec" => Ok(Self::SshExec),
            _ => bail!("invalid peer protocol {value:?}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerEndpoint {
    pub protocol: PeerProtocol,
    pub addr: Option<SocketAddr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerHello {
    pub version: u16,
    pub node: String,
    pub protocols: Vec<PeerProtocol>,
    pub features: Vec<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub feature_bits: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerWelcome {
    pub version: u16,
    pub node: String,
    pub accepted: Option<PeerProtocol>,
    pub features: Vec<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub feature_bits: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    pub message: String,
}

pub fn default_features() -> Vec<String> {
    [
        "frames-v1",
        "socks5h",
        "tcp-connect",
        "udp-associate",
        "ssh-native-direct-tcpip",
        "quic-native-streams-v1",
    ]
    .into_iter()
    .map(ToOwned::to_owned)
    .collect()
}

pub fn default_feature_bits() -> Map<String, Value> {
    default_features()
        .into_iter()
        .map(|feature| (feature, Value::Bool(true)))
        .collect()
}

pub fn select_supported_protocol(
    requested: &[PeerProtocol],
    supported: &[PeerProtocol],
) -> Option<PeerProtocol> {
    requested
        .iter()
        .copied()
        .find(|protocol| supported.contains(protocol))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_handshake_models_keep_wire_json_shape() {
        let hello = PeerHello {
            version: PEER_VERSION,
            node: "client".to_string(),
            protocols: vec![PeerProtocol::SshDirect],
            features: default_features(),
            feature_bits: default_feature_bits(),
            binary_version: Some("0.1.1".to_string()),
            os: Some("linux".to_string()),
            arch: Some("x86_64".to_string()),
        };
        let value = serde_json::to_value(&hello).unwrap();

        assert_eq!(value["version"], 1);
        assert_eq!(value["node"], "client");
        assert_eq!(value["protocols"][0], "ssh-direct");
        assert_eq!(value["features"][0], "frames-v1");
        assert_eq!(value["feature_bits"]["frames-v1"], true);

        let decoded: PeerHello = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, hello);
    }

    #[test]
    fn protocol_parser_accepts_operational_aliases() {
        assert_eq!(
            "exec".parse::<PeerProtocol>().unwrap(),
            PeerProtocol::SshExec
        );
        assert_eq!(
            "ssh-tcp".parse::<PeerProtocol>().unwrap(),
            PeerProtocol::SshDirect
        );
        assert_eq!("tls".parse::<PeerProtocol>().unwrap(), PeerProtocol::TlsTcp);
        assert_eq!(
            "direct-tcp".parse::<PeerProtocol>().unwrap(),
            PeerProtocol::Tcp
        );
        assert_eq!(
            "native-quic".parse::<PeerProtocol>().unwrap(),
            PeerProtocol::QuicNative
        );
    }

    #[test]
    fn supported_protocol_selection_preserves_client_preference() {
        let requested = [PeerProtocol::QuicNative, PeerProtocol::Quic];
        let supported = [PeerProtocol::Quic, PeerProtocol::QuicNative];

        assert_eq!(
            select_supported_protocol(&requested, &supported),
            Some(PeerProtocol::QuicNative)
        );
        assert_eq!(
            select_supported_protocol(&[PeerProtocol::Quic], &supported),
            Some(PeerProtocol::Quic)
        );
        assert_eq!(
            select_supported_protocol(&[PeerProtocol::SshExec], &supported),
            None
        );
    }
}
