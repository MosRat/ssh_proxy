use std::net::SocketAddr;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::version::{ControlApiVersion, FeatureSet, PeerProtocolVersion};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PeerDescriptor {
    pub(crate) ok: Option<bool>,
    pub(crate) kind: Option<String>,
    pub(crate) source: Option<String>,
    pub(crate) schema_version: Option<u32>,
    pub(crate) node_id: Option<String>,
    pub(crate) node_name: Option<String>,
    pub(crate) service_instance_id: Option<String>,
    pub(crate) version: Option<String>,
    pub(crate) os: Option<String>,
    pub(crate) arch: Option<String>,
    pub(crate) os_user: Option<String>,
    pub(crate) data_dir: Option<String>,
    pub(crate) target: Option<String>,
    pub(crate) control_endpoint: Option<String>,
    pub(crate) control_api_version: Option<u16>,
    pub(crate) peer_protocol_version: Option<u16>,
    pub(crate) features: Vec<String>,
    pub(crate) feature_bits: serde_json::Map<String, Value>,
    pub(crate) endpoints: PeerDescriptorEndpoints,
    pub(crate) transport_protocols: Vec<String>,
    pub(crate) auth: PeerDescriptorAuth,
    pub(crate) routes_path: Option<Value>,
    pub(crate) route_autostart: Option<bool>,
}

impl PeerDescriptor {
    pub(crate) fn from_value(value: Value) -> Result<Self> {
        if value.get("ok").and_then(Value::as_bool) == Some(false) {
            bail!("descriptor reports ok=false");
        }
        serde_json::from_value(value).context("failed to parse peer descriptor")
    }

    pub(crate) fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub(crate) fn control_endpoint(&self) -> Option<String> {
        self.endpoints
            .control
            .clone()
            .or_else(|| self.control_endpoint.clone())
    }

    pub(crate) fn transport_addr(&self) -> Option<SocketAddr> {
        self.endpoints
            .transport
            .as_deref()
            .and_then(parse_socket_or_tcp_endpoint)
    }

    pub(crate) fn tls_transport_addr(&self) -> Option<SocketAddr> {
        self.endpoints
            .tls_transport
            .as_deref()
            .and_then(parse_socket_or_tcp_endpoint)
    }

    pub(crate) fn quic_transport_addr(&self) -> Option<SocketAddr> {
        self.endpoints
            .quic_transport
            .as_deref()
            .and_then(parse_socket_or_tcp_endpoint)
    }

    pub(crate) fn control_addr(&self) -> Option<SocketAddr> {
        self.control_endpoint()
            .as_deref()
            .and_then(parse_socket_or_tcp_endpoint)
    }

    pub(crate) fn transport_protocols_or_infer(&self) -> Vec<String> {
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

    pub(crate) fn feature_set(&self) -> FeatureSet {
        FeatureSet::from_parts(self.features.clone(), self.feature_bits.clone())
    }

    pub(crate) fn control_version(&self) -> Option<ControlApiVersion> {
        self.control_api_version.map(ControlApiVersion::new)
    }

    pub(crate) fn peer_version(&self) -> Option<PeerProtocolVersion> {
        self.peer_protocol_version.map(PeerProtocolVersion::new)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PeerDescriptorEndpoints {
    pub(crate) control: Option<String>,
    pub(crate) transport: Option<String>,
    pub(crate) tls_transport: Option<String>,
    pub(crate) quic_transport: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PeerDescriptorAuth {
    pub(crate) control_token: Option<bool>,
    pub(crate) transport_token: Option<bool>,
    pub(crate) token_metadata: Option<Value>,
    pub(crate) token_generation: Option<u64>,
    pub(crate) tls_server_cert: Option<bool>,
    pub(crate) tls_client_ca: Option<bool>,
    pub(crate) tls_server_cert_fingerprint: Option<String>,
    pub(crate) tls_client_ca_fingerprint: Option<String>,
}

pub(crate) fn parse_socket_or_tcp_endpoint(value: &str) -> Option<SocketAddr> {
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
