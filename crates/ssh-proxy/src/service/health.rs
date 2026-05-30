use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::Path,
};

use serde_json::Value;
use ssh_proxy_service::{
    BinaryHealthInput, ConfigFileHealthInput, ConfigFileHealthState, EndpointHealthInput,
    PeerCompatibilityInput, PeerHealthInput, PeerRegistryHealthInput, RouteStoreHealthInput,
    RouteStoreHealthState, ServiceHealthInput, binary_health_report, config_file_health_report,
    endpoint_health_report, peer_compatibility_report, peer_health_report,
    peer_registry_health_report, route_store_health_report, service_health_report,
};
use tokio::net::TcpStream;
use tokio::time::{self, Duration};

use crate::config;

use super::plan::ServicePlan;

pub(super) async fn service_health(plan: &ServicePlan) -> Value {
    let (config_health, config_snapshot) = config_file_health_with_snapshot(&plan.config_path);
    service_health_report(ServiceHealthInput {
        config: config_health,
        route_store: route_store_health(&plan.route_store_path),
        binary: binary_health(plan),
        control: control_endpoint_health(&plan.endpoint),
        plain_tcp: tcp_listener_health(plan.transport).await,
        tls_tcp: tcp_listener_health(plan.tls_transport).await,
        quic: quic_listener_health(plan.quic_transport),
        peers: peer_registry_health(config_snapshot.as_ref()).await,
    })
}

pub(super) fn config_file_health_with_snapshot(path: &Path) -> (Value, Option<config::AppConfig>) {
    if !path.exists() {
        return (
            config_file_health_report(ConfigFileHealthInput {
                path: path.display().to_string(),
                current_schema_version: config::CONFIG_SCHEMA_VERSION,
                state: ConfigFileHealthState::Missing,
            }),
            None,
        );
    }
    match std::fs::read_to_string(path).map_err(anyhow::Error::from) {
        Ok(text) => match toml::from_str::<config::AppConfig>(&text)
            .map_err(anyhow::Error::from)
            .and_then(|config| {
                config.validate_schema()?;
                Ok(config)
            }) {
            Ok(config) => {
                let raw_schema = toml::from_str::<toml::Value>(&text).ok().and_then(|value| {
                    value
                        .get("schema_version")
                        .and_then(|value| value.as_integer())
                });
                let health = config_file_health_report(ConfigFileHealthInput {
                    path: path.display().to_string(),
                    current_schema_version: config::CONFIG_SCHEMA_VERSION,
                    state: ConfigFileHealthState::Valid {
                        schema_version: config.schema_version,
                        schema_recorded: raw_schema.is_some(),
                    },
                });
                (health, Some(config))
            }
            Err(err) => (
                config_file_health_report(ConfigFileHealthInput {
                    path: path.display().to_string(),
                    current_schema_version: config::CONFIG_SCHEMA_VERSION,
                    state: ConfigFileHealthState::Invalid {
                        error: err.to_string(),
                    },
                }),
                None,
            ),
        },
        Err(err) => (
            config_file_health_report(ConfigFileHealthInput {
                path: path.display().to_string(),
                current_schema_version: config::CONFIG_SCHEMA_VERSION,
                state: ConfigFileHealthState::ReadError {
                    exists: path.exists(),
                    error: err.to_string(),
                },
            }),
            None,
        ),
    }
}

pub(super) fn route_store_health(path: &Path) -> Value {
    if !path.exists() {
        return route_store_health_report(RouteStoreHealthInput {
            path: path.display().to_string(),
            current_version: 1,
            state: RouteStoreHealthState::Missing,
        });
    }
    match std::fs::read_to_string(path)
        .map_err(anyhow::Error::from)
        .and_then(|text| serde_json::from_str::<Value>(&text).map_err(anyhow::Error::from))
    {
        Ok(value) => route_store_health_report(RouteStoreHealthInput {
            path: path.display().to_string(),
            current_version: 1,
            state: RouteStoreHealthState::Loaded(value),
        }),
        Err(err) => route_store_health_report(RouteStoreHealthInput {
            path: path.display().to_string(),
            current_version: 1,
            state: RouteStoreHealthState::Error {
                exists: true,
                error: err.to_string(),
            },
        }),
    }
}

pub(super) async fn peer_registry_health(config: Option<&config::AppConfig>) -> Value {
    let Some(config) = config else {
        return peer_registry_health_report(PeerRegistryHealthInput {
            config_available: false,
            peers: Vec::new(),
        });
    };
    let mut peers = config.peers.iter().collect::<Vec<_>>();
    peers.sort_by(|(left, _), (right, _)| left.cmp(right));
    let mut summaries = Vec::with_capacity(peers.len());
    for (alias, peer) in peers {
        let summary = peer_health(alias, peer).await;
        summaries.push(summary);
    }
    peer_registry_health_report(PeerRegistryHealthInput {
        config_available: true,
        peers: summaries,
    })
}

pub(super) fn local_probe_addr(addr: SocketAddr) -> SocketAddr {
    if !addr.ip().is_unspecified() {
        return addr;
    }
    match addr.ip() {
        IpAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), addr.port()),
        IpAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), addr.port()),
    }
}

fn binary_health(plan: &ServicePlan) -> Value {
    binary_health_report(BinaryHealthInput {
        source_exists: plan.source_exe.exists(),
        installed_exists: plan.exe.exists(),
        copy_exe: plan.copy_exe,
        same_path: plan.source_exe == plan.exe,
    })
}

async fn peer_health(alias: &str, peer: &config::PeerRecord) -> Value {
    let control = optional_control_endpoint_health(peer.control_endpoint.as_deref());
    let plain_tcp = tcp_listener_health(peer.transport).await;
    let tls_tcp = tcp_listener_health(peer.tls_transport).await;
    let quic = quic_listener_health(peer.quic_transport);
    let compatibility = peer_compatibility_report(PeerCompatibilityInput {
        local_version: env!("CARGO_PKG_VERSION").to_string(),
        local_control_api_version: crate::node_daemon::control_api_version(),
        local_peer_protocol_version: crate::node_daemon::peer_protocol_version(),
        local_features: crate::node_daemon::peer_protocol_features(),
        remote_version: peer.version.clone(),
        remote_control_api_version: peer.control_api_version,
        remote_peer_protocol_version: peer.peer_protocol_version,
        remote_features: peer.features.clone(),
        remote_os: peer.os.clone(),
        remote_arch: peer.arch.clone(),
    });
    peer_health_report(PeerHealthInput {
        alias: alias.to_string(),
        node_id: peer.node_id.clone(),
        node_name: peer.node_name.clone(),
        version: peer.version.clone(),
        control_api_version: peer.control_api_version,
        peer_protocol_version: peer.peer_protocol_version,
        features: peer.features.clone(),
        os: peer.os.clone(),
        arch: peer.arch.clone(),
        target: peer.target.clone(),
        trust: peer.trust.clone(),
        protocols: peer.known_transport_protocols(),
        token_present: peer.token.is_some(),
        token_metadata: serde_json::to_value(&peer.token_metadata).unwrap_or(Value::Null),
        compatibility,
        last_seen_unix: peer.last_seen_unix,
        control,
        plain_tcp,
        tls_tcp,
        quic,
    })
}

fn control_endpoint_health(endpoint: &str) -> Value {
    match crate::control_socket::ControlEndpoint::parse(endpoint) {
        Ok(parsed) => endpoint_health_report(EndpointHealthInput::Control {
            endpoint: endpoint.to_string(),
            kind: Some(control_endpoint_kind(&parsed).to_string()),
            error: None,
        }),
        Err(err) => endpoint_health_report(EndpointHealthInput::Control {
            endpoint: endpoint.to_string(),
            kind: None,
            error: Some(err.to_string()),
        }),
    }
}

fn optional_control_endpoint_health(endpoint: Option<&str>) -> Value {
    match endpoint {
        Some(endpoint) => control_endpoint_health(endpoint),
        None => endpoint_health_report(EndpointHealthInput::MissingPeerControl),
    }
}

fn control_endpoint_kind(endpoint: &crate::control_socket::ControlEndpoint) -> &'static str {
    match endpoint {
        crate::control_socket::ControlEndpoint::Tcp(_) => "tcp",
        #[cfg(unix)]
        crate::control_socket::ControlEndpoint::Unix(_) => "unix",
        #[cfg(windows)]
        crate::control_socket::ControlEndpoint::NamedPipe(_) => "named-pipe",
    }
}

async fn tcp_listener_health(addr: Option<SocketAddr>) -> Value {
    let Some(addr) = addr else {
        return endpoint_health_report(EndpointHealthInput::Tcp {
            addr: None,
            probe_addr: None,
            reachable: None,
            error: None,
        });
    };
    let probe_addr = local_probe_addr(addr);
    match time::timeout(Duration::from_millis(500), TcpStream::connect(probe_addr)).await {
        Ok(Ok(stream)) => {
            drop(stream);
            endpoint_health_report(EndpointHealthInput::Tcp {
                addr: Some(addr.to_string()),
                probe_addr: Some(probe_addr.to_string()),
                reachable: Some(true),
                error: None,
            })
        }
        Ok(Err(err)) => endpoint_health_report(EndpointHealthInput::Tcp {
            addr: Some(addr.to_string()),
            probe_addr: Some(probe_addr.to_string()),
            reachable: Some(false),
            error: Some(err.to_string()),
        }),
        Err(_) => endpoint_health_report(EndpointHealthInput::Tcp {
            addr: Some(addr.to_string()),
            probe_addr: Some(probe_addr.to_string()),
            reachable: Some(false),
            error: Some("TCP listener probe timed out after 500 ms".to_string()),
        }),
    }
}

fn quic_listener_health(addr: Option<SocketAddr>) -> Value {
    endpoint_health_report(EndpointHealthInput::Quic {
        addr: addr.map(|addr| addr.to_string()),
    })
}
