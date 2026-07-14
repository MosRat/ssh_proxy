use std::net::SocketAddr;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::version::{ControlApiVersion, FeatureSet, PeerProtocolVersion};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PeerDescriptor {
    pub ok: Option<bool>,
    pub kind: Option<String>,
    pub source: Option<String>,
    pub schema_version: Option<u32>,
    pub node_id: Option<String>,
    pub node_name: Option<String>,
    pub service_instance_id: Option<String>,
    pub version: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub os_user: Option<String>,
    pub data_dir: Option<String>,
    pub target: Option<String>,
    pub control_endpoint: Option<String>,
    pub control_api_version: Option<u16>,
    pub peer_protocol_version: Option<u16>,
    pub features: Vec<String>,
    pub feature_bits: serde_json::Map<String, Value>,
    pub endpoints: PeerDescriptorEndpoints,
    pub transport_protocols: Vec<String>,
    pub auth: PeerDescriptorAuth,
    pub routes_path: Option<Value>,
    pub route_autostart: Option<bool>,
}

impl PeerDescriptor {
    pub fn from_value(value: Value) -> Result<Self> {
        if value.get("ok").and_then(Value::as_bool) == Some(false) {
            bail!("descriptor reports ok=false");
        }
        serde_json::from_value(value).context("failed to parse peer descriptor")
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn control_endpoint(&self) -> Option<String> {
        self.endpoints
            .control
            .clone()
            .or_else(|| self.control_endpoint.clone())
    }

    pub fn transport_addr(&self) -> Option<SocketAddr> {
        self.endpoints
            .transport
            .as_deref()
            .and_then(parse_socket_or_tcp_endpoint)
    }

    pub fn tls_transport_addr(&self) -> Option<SocketAddr> {
        self.endpoints
            .tls_transport
            .as_deref()
            .and_then(parse_socket_or_tcp_endpoint)
    }

    pub fn quic_transport_addr(&self) -> Option<SocketAddr> {
        self.endpoints
            .quic_transport
            .as_deref()
            .and_then(parse_socket_or_tcp_endpoint)
    }

    pub fn control_addr(&self) -> Option<SocketAddr> {
        self.control_endpoint()
            .as_deref()
            .and_then(parse_socket_or_tcp_endpoint)
    }

    pub fn transport_protocols_or_infer(&self) -> Vec<String> {
        if !self.transport_protocols.is_empty() {
            return self.transport_protocols.clone();
        }
        let mut protocols = Vec::new();
        if self.quic_transport_addr().is_some() {
            protocols.push("quic".to_string());
        }
        if self.tls_transport_addr().is_some() {
            protocols.push("tls-tcp".to_string());
        }
        if self.transport_addr().is_some() {
            protocols.push("plain-tcp".to_string());
        }
        protocols
    }

    pub fn feature_set(&self) -> FeatureSet {
        FeatureSet::from_parts(self.features.clone(), self.feature_bits.clone())
    }

    pub fn control_version(&self) -> Option<ControlApiVersion> {
        self.control_api_version.map(ControlApiVersion::new)
    }

    pub fn peer_version(&self) -> Option<PeerProtocolVersion> {
        self.peer_protocol_version.map(PeerProtocolVersion::new)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PeerDescriptorEndpoints {
    pub control: Option<String>,
    pub transport: Option<String>,
    pub tls_transport: Option<String>,
    pub quic_transport: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PeerDescriptorAuth {
    pub control_token: Option<bool>,
    pub transport_token: Option<bool>,
    pub token_metadata: Option<Value>,
    pub token_generation: Option<u64>,
    pub tls_server_cert: Option<bool>,
    pub tls_client_ca: Option<bool>,
    pub tls_server_cert_fingerprint: Option<String>,
    pub tls_client_ca_fingerprint: Option<String>,
}

pub fn parse_socket_or_tcp_endpoint(value: &str) -> Option<SocketAddr> {
    value.strip_prefix("tcp://").unwrap_or(value).parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn peer_descriptor_accepts_current_export_shape() {
        let descriptor = PeerDescriptor::from_value(json!({
            "ok": true,
            "kind": "peer_descriptor",
            "node_id": "spx-remote",
            "node_name": "remote",
            "version": "0.1.1",
            "control_api_version": 1,
            "peer_protocol_version": 1,
            "features": ["frames-v1", "quic-native-streams-v1"],
            "feature_bits": {
                "frames-v1": true
            },
            "endpoints": {
                "control": "tcp://127.0.0.1:19081",
                "transport": "127.0.0.1:19080",
                "tls_transport": "127.0.0.1:19082",
                "quic_transport": "127.0.0.1:19083"
            },
            "transport_protocols": ["quic", "tls-tcp", "plain-tcp"],
            "auth": {
                "control_token": true,
                "tls_server_cert_fingerprint": "sha256:abc"
            }
        }))
        .unwrap();

        assert_eq!(
            descriptor.control_addr(),
            Some("127.0.0.1:19081".parse().unwrap())
        );
        assert_eq!(
            descriptor.transport_protocols_or_infer(),
            vec!["quic", "tls-tcp", "plain-tcp"]
        );
        assert_eq!(
            descriptor.control_version(),
            Some(ControlApiVersion::current())
        );
        assert_eq!(
            descriptor.peer_version(),
            Some(PeerProtocolVersion::current())
        );
        assert_eq!(descriptor.feature_set().features[0], "frames-v1");
        assert_eq!(
            descriptor.auth.tls_server_cert_fingerprint.as_deref(),
            Some("sha256:abc")
        );
    }

    #[test]
    fn peer_descriptor_infers_protocols_from_endpoints() {
        let descriptor = PeerDescriptor::from_value(json!({
            "ok": true,
            "endpoints": {
                "transport": "127.0.0.1:19080",
                "tls_transport": "127.0.0.1:19082"
            }
        }))
        .unwrap();

        assert_eq!(
            descriptor.transport_protocols_or_infer(),
            vec!["tls-tcp", "plain-tcp"]
        );
    }

    #[test]
    fn peer_descriptor_rejects_not_ok_payload() {
        let err = PeerDescriptor::from_value(json!({
            "ok": false,
            "error": "not ready"
        }))
        .unwrap_err()
        .to_string();

        assert!(err.contains("ok=false"));
    }
}
