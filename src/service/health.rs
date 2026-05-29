use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::Path,
};

use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::time::{self, Duration};

use crate::config;

use super::{peer_health, plan::ServicePlan};

pub(super) async fn service_health(plan: &ServicePlan) -> Value {
    let (config_health, config_snapshot) = config_file_health_with_snapshot(&plan.config_path);
    json!({
        "config": config_health,
        "route_store": route_store_health(&plan.route_store_path),
        "binary": binary_health(plan),
        "listeners": {
            "control": control_endpoint_health(&plan.endpoint),
            "plain_tcp": tcp_listener_health(plan.transport).await,
            "tls_tcp": tcp_listener_health(plan.tls_transport).await,
            "quic": quic_listener_health(plan.quic_transport),
        },
        "peers": peer_registry_health(config_snapshot.as_ref()).await,
    })
}

pub(super) fn config_file_health_with_snapshot(path: &Path) -> (Value, Option<config::AppConfig>) {
    if !path.exists() {
        return (
            json!({
            "ok": false,
            "exists": false,
            "path": path,
            "message": "config file does not exist; run `ssh_proxy config init` or `ssh_proxy service install`"
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
                let health = json!({
                        "ok": true,
                        "exists": true,
                        "path": path,
                        "schema_version": config.schema_version,
                        "current_schema_version": config::CONFIG_SCHEMA_VERSION,
                        "schema_recorded": raw_schema.is_some(),
                        "legacy_without_schema": raw_schema.is_none(),
                });
                (health, Some(config))
            }
            Err(err) => (
                json!({
                    "ok": false,
                    "exists": true,
                    "path": path,
                    "current_schema_version": config::CONFIG_SCHEMA_VERSION,
                    "error": err.to_string(),
                }),
                None,
            ),
        },
        Err(err) => (
            json!({
                "ok": false,
                "exists": path.exists(),
                "path": path,
                "error": err.to_string(),
            }),
            None,
        ),
    }
}

pub(super) fn route_store_health(path: &Path) -> Value {
    if !path.exists() {
        return json!({
            "ok": true,
            "exists": false,
            "path": path,
            "routes": 0,
            "message": "route store does not exist yet"
        });
    }
    match std::fs::read_to_string(path)
        .map_err(anyhow::Error::from)
        .and_then(|text| serde_json::from_str::<Value>(&text).map_err(anyhow::Error::from))
    {
        Ok(value) => {
            let version = value.get("version").and_then(Value::as_u64);
            let routes_array = value.get("routes").and_then(Value::as_array);
            let routes = routes_array.map(Vec::len);
            let duplicate_ids = duplicate_route_ids(routes_array);
            let ok = version == Some(1) && routes.is_some() && duplicate_ids.is_empty();
            json!({
                "ok": ok,
                "exists": true,
                "path": path,
                "version": version,
                "current_version": 1,
                "routes": routes,
                "duplicate_ids": duplicate_ids,
            })
        }
        Err(err) => json!({
            "ok": false,
            "exists": true,
            "path": path,
            "error": err.to_string(),
        }),
    }
}

pub(super) async fn peer_registry_health(config: Option<&config::AppConfig>) -> Value {
    let Some(config) = config else {
        return json!({
            "ok": false,
            "count": 0,
            "error": "config unavailable",
            "peers": [],
        });
    };
    let mut peers = config.peers.iter().collect::<Vec<_>>();
    peers.sort_by(|(left, _), (right, _)| left.cmp(right));
    let mut summaries = Vec::with_capacity(peers.len());
    let mut ok = true;
    for (alias, peer) in peers {
        let summary = peer_health(alias, peer).await;
        ok &= summary.get("ok").and_then(Value::as_bool).unwrap_or(false);
        summaries.push(summary);
    }
    json!({
        "ok": ok,
        "count": summaries.len(),
        "peers": summaries,
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

fn duplicate_route_ids(routes: Option<&Vec<Value>>) -> Vec<String> {
    let Some(routes) = routes else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut duplicates = Vec::new();
    for route in routes {
        let Some(id) = route.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !seen.insert(id.to_string()) && !duplicates.iter().any(|duplicate| duplicate == id) {
            duplicates.push(id.to_string());
        }
    }
    duplicates
}

fn binary_health(plan: &ServicePlan) -> Value {
    json!({
        "source_exists": plan.source_exe.exists(),
        "installed_exists": plan.exe.exists(),
        "copy_exe": plan.copy_exe,
        "same_path": plan.source_exe == plan.exe,
    })
}

async fn peer_health(alias: &str, peer: &config::PeerRecord) -> Value {
    let control = optional_control_endpoint_health(peer.control_endpoint.as_deref());
    let plain_tcp = tcp_listener_health(peer.transport).await;
    let tls_tcp = tcp_listener_health(peer.tls_transport).await;
    let quic = quic_listener_health(peer.quic_transport);
    let compatibility = peer_health::compatibility(peer);
    let ok = control.get("ok").and_then(Value::as_bool).unwrap_or(false)
        && plain_tcp
            .get("ok")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        && tls_tcp.get("ok").and_then(Value::as_bool).unwrap_or(false)
        && quic.get("ok").and_then(Value::as_bool).unwrap_or(false)
        && compatibility
            .get("ok")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    json!({
        "ok": ok,
        "alias": alias,
        "node_id": peer.node_id,
        "node_name": peer.node_name,
        "version": peer.version,
        "control_api_version": peer.control_api_version,
        "peer_protocol_version": peer.peer_protocol_version,
        "features": peer.features,
        "os": peer.os,
        "arch": peer.arch,
        "target": peer.target,
        "trust": peer.trust,
        "protocols": peer.known_transport_protocols(),
        "auth": {
            "token": peer.token.is_some(),
            "token_metadata": peer.token_metadata,
        },
        "compatibility": compatibility,
        "last_seen_unix": peer.last_seen_unix,
        "endpoints": {
            "control": control,
            "plain_tcp": plain_tcp,
            "tls_tcp": tls_tcp,
            "quic": quic,
        }
    })
}

fn control_endpoint_health(endpoint: &str) -> Value {
    match crate::control_socket::ControlEndpoint::parse(endpoint) {
        Ok(parsed) => json!({
            "ok": true,
            "endpoint": endpoint,
            "kind": control_endpoint_kind(&parsed),
        }),
        Err(err) => json!({
            "ok": false,
            "endpoint": endpoint,
            "error": err.to_string(),
        }),
    }
}

fn optional_control_endpoint_health(endpoint: Option<&str>) -> Value {
    match endpoint {
        Some(endpoint) => control_endpoint_health(endpoint),
        None => json!({
            "ok": false,
            "configured": false,
            "reachable": Value::Null,
            "message": "peer control endpoint is not recorded",
        }),
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
        return json!({
            "ok": true,
            "configured": false,
            "reachable": Value::Null,
        });
    };
    let probe_addr = local_probe_addr(addr);
    match time::timeout(Duration::from_millis(500), TcpStream::connect(probe_addr)).await {
        Ok(Ok(stream)) => {
            drop(stream);
            json!({
                "ok": true,
                "configured": true,
                "addr": addr.to_string(),
                "probe_addr": probe_addr.to_string(),
                "reachable": true,
            })
        }
        Ok(Err(err)) => json!({
            "ok": false,
            "configured": true,
            "addr": addr.to_string(),
            "probe_addr": probe_addr.to_string(),
            "reachable": false,
            "error": err.to_string(),
        }),
        Err(_) => json!({
            "ok": false,
            "configured": true,
            "addr": addr.to_string(),
            "probe_addr": probe_addr.to_string(),
            "reachable": false,
            "error": "TCP listener probe timed out after 500 ms",
        }),
    }
}

fn quic_listener_health(addr: Option<SocketAddr>) -> Value {
    match addr {
        Some(addr) => json!({
            "ok": true,
            "configured": true,
            "addr": addr.to_string(),
            "reachable": Value::Null,
            "message": "QUIC UDP listener reachability is not probed by service status yet",
        }),
        None => json!({
            "ok": true,
            "configured": false,
            "reachable": Value::Null,
        }),
    }
}
