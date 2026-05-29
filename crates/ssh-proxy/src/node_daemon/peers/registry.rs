use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::config;

use super::compatibility;

pub(super) fn peers_response(config: &config::AppConfig) -> Value {
    let mut peers = config.peers.iter().collect::<Vec<_>>();
    peers.sort_by(|(left, _), (right, _)| left.cmp(right));
    let peers = peers
        .into_iter()
        .map(|(alias, peer)| {
            json!({
                "alias": alias,
                "node_id": peer.node_id,
                "node_name": peer.node_name,
                "service_instance_id": peer.service_instance_id,
                "version": peer.version,
                "control_api_version": peer.control_api_version,
                "peer_protocol_version": peer.peer_protocol_version,
                "features": peer.features,
                "os": peer.os,
                "arch": peer.arch,
                "os_user": peer.os_user,
                "data_dir": peer.data_dir,
                "target": peer.target,
                "trust": peer.trust,
                "remote_path": peer.remote_path,
                "control_endpoint": peer.control_endpoint,
                "transport": peer.transport.map(|addr| addr.to_string()),
                "tls_transport": peer.tls_transport.map(|addr| addr.to_string()),
                "quic_transport": peer.quic_transport.map(|addr| addr.to_string()),
                "transport_protocols": peer.known_transport_protocols(),
                "auth": {
                    "token": peer.token.is_some(),
                    "token_metadata": peer.token_metadata.clone(),
                    "token_generation": peer.token_metadata.as_ref().map(|metadata| metadata.generation),
                    "tls_server_cert_fingerprint": peer.tls_server_cert_fingerprint,
                    "tls_client_ca_fingerprint": peer.tls_client_ca_fingerprint,
                },
                "compatibility": compatibility::build_saved_peer_version_check(alias, peer),
                "last_seen_unix": peer.last_seen_unix,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "ok": true,
        "node_id": config.identity.node_id,
        "node_name": config.identity.node_name,
        "peers": peers,
    })
}

pub(super) fn remove_peer(config: &mut config::AppConfig, alias: &str) -> Result<()> {
    if config.peers.remove(alias).is_none() {
        bail!("peer {alias:?} is not recorded");
    }
    Ok(())
}

pub(super) fn peer_is_route_ready(config: &config::AppConfig, target: &str) -> bool {
    config.peers.get(target).is_some_and(|peer| {
        peer.remote_path.is_some()
            && peer.control_endpoint.is_some()
            && (peer.transport.is_some()
                || peer.tls_transport.is_some()
                || peer.quic_transport.is_some())
    })
}
