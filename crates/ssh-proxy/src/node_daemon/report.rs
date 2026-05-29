use std::{net::SocketAddr, path::PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::{config, peer_transport};

#[derive(Debug, Clone)]
pub(super) struct NodeDescriptorReport {
    pub(super) name: String,
    pub(super) node_id: Option<String>,
    pub(super) node_name: String,
    pub(super) service_instance_id: String,
    pub(super) os_user: String,
    pub(super) data_dir: Option<PathBuf>,
    pub(super) control_api_version: u16,
    pub(super) peer_protocol_version: u16,
    pub(super) features: Vec<String>,
    pub(super) feature_bits: serde_json::Map<String, Value>,
    pub(super) control_endpoint: String,
    pub(super) endpoints: NodeDescriptorEndpoints,
    pub(super) transport_protocols: Vec<String>,
    pub(super) quic_transport_options: peer_transport::QuicTransportOptions,
    pub(super) quic_runtime: peer_transport::QuicRuntimeDiagnostics,
    pub(super) auth: NodeDescriptorAuth,
    pub(super) routes_path: PathBuf,
    pub(super) route_autostart: bool,
    pub(super) linux_musl_sidecar: Value,
}

impl NodeDescriptorReport {
    pub(super) fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct NodeDescriptorEndpoints {
    pub(super) control: String,
    pub(super) transport: Option<SocketAddr>,
    pub(super) tls_transport: Option<SocketAddr>,
    pub(super) quic_transport: Option<SocketAddr>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct NodeDescriptorAuth {
    pub(super) control_token: bool,
    pub(super) transport_token: bool,
    pub(super) token_metadata: Option<config::TokenMetadata>,
    pub(super) token_generation: Option<u64>,
    pub(super) tls_server_cert: bool,
    pub(super) tls_client_ca: bool,
    pub(super) tls_server_cert_fingerprint: Option<String>,
    pub(super) tls_client_ca_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct PublicNodeDescriptorReport<'a> {
    ok: bool,
    kind: &'static str,
    name: &'a str,
    node_id: &'a Option<String>,
    node_name: &'a str,
    service_instance_id: &'a str,
    version: &'static str,
    os: &'static str,
    arch: &'static str,
    os_user: &'a str,
    data_dir: &'a Option<PathBuf>,
    control_api_version: u16,
    peer_protocol_version: u16,
    features: &'a [String],
    feature_bits: &'a serde_json::Map<String, Value>,
    control_endpoint: &'a str,
    endpoints: &'a NodeDescriptorEndpoints,
    transport_protocols: &'a [String],
    quic_transport_options: peer_transport::QuicTransportOptions,
    quic_runtime: &'a peer_transport::QuicRuntimeDiagnostics,
    auth: &'a NodeDescriptorAuth,
    routes_path: &'a PathBuf,
    route_autostart: bool,
    linux_musl_sidecar: &'a Value,
}

impl Serialize for NodeDescriptorReport {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        PublicNodeDescriptorReport {
            ok: true,
            kind: "peer_descriptor",
            name: &self.name,
            node_id: &self.node_id,
            node_name: &self.node_name,
            service_instance_id: &self.service_instance_id,
            version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            os_user: &self.os_user,
            data_dir: &self.data_dir,
            control_api_version: self.control_api_version,
            peer_protocol_version: self.peer_protocol_version,
            features: &self.features,
            feature_bits: &self.feature_bits,
            control_endpoint: &self.control_endpoint,
            endpoints: &self.endpoints,
            transport_protocols: &self.transport_protocols,
            quic_transport_options: self.quic_transport_options,
            quic_runtime: &self.quic_runtime,
            auth: &self.auth,
            routes_path: &self.routes_path,
            route_autostart: self.route_autostart,
            linux_musl_sidecar: &self.linux_musl_sidecar,
        }
        .serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol_core::peer::{default_feature_bits, default_features};

    #[test]
    fn node_descriptor_report_preserves_public_shape() {
        let report = NodeDescriptorReport {
            name: "local".to_string(),
            node_id: Some("spx-local".to_string()),
            node_name: "local-node".to_string(),
            service_instance_id: "spx-local@user:tcp://127.0.0.1:19081".to_string(),
            os_user: "user".to_string(),
            data_dir: Some(PathBuf::from("/tmp/ssh_proxy")),
            control_api_version: 1,
            peer_protocol_version: 1,
            features: default_features(),
            feature_bits: default_feature_bits(),
            control_endpoint: "tcp://127.0.0.1:19081".to_string(),
            endpoints: NodeDescriptorEndpoints {
                control: "tcp://127.0.0.1:19081".to_string(),
                transport: Some("127.0.0.1:19080".parse().unwrap()),
                tls_transport: None,
                quic_transport: None,
            },
            transport_protocols: vec!["plain-tcp".to_string()],
            quic_transport_options: peer_transport::QuicTransportOptions::default(),
            quic_runtime: peer_transport::quic_runtime_diagnostics(
                peer_transport::QuicTransportOptions::default(),
            ),
            auth: NodeDescriptorAuth {
                control_token: true,
                transport_token: true,
                token_metadata: Some(config::TokenMetadata::new("daemon-control-transport")),
                token_generation: Some(1),
                tls_server_cert: false,
                tls_client_ca: false,
                tls_server_cert_fingerprint: None,
                tls_client_ca_fingerprint: None,
            },
            routes_path: PathBuf::from("routes.json"),
            route_autostart: true,
            linux_musl_sidecar: serde_json::json!({"available": true}),
        };

        let value = report.to_value();

        assert_eq!(value["ok"], true);
        assert_eq!(value["kind"], "peer_descriptor");
        assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["endpoints"]["control"], "tcp://127.0.0.1:19081");
        assert_eq!(value["transport_protocols"][0], "plain-tcp");
        assert_eq!(value["auth"]["control_token"], true);
    }
}
