use serde_json::{Value, json};

use crate::{config, node_daemon};

pub(super) fn compatibility(peer: &config::PeerRecord) -> Value {
    let local_version = env!("CARGO_PKG_VERSION");
    let local_control = node_daemon::control_api_version();
    let local_peer = node_daemon::peer_protocol_version();
    let local_features = node_daemon::peer_protocol_features();
    let missing_features = local_features
        .iter()
        .filter(|feature| !peer.features.contains(*feature))
        .cloned()
        .collect::<Vec<_>>();
    let common_features = local_features
        .iter()
        .filter(|feature| peer.features.contains(*feature))
        .cloned()
        .collect::<Vec<_>>();

    let checks = vec![
        control_api_check(local_control, peer.control_api_version),
        peer_protocol_check(local_peer, peer.peer_protocol_version),
        feature_check(&local_features, &peer.features, &missing_features),
        binary_version_check(local_version, peer.version.as_deref()),
    ];
    let compatible = checks
        .iter()
        .all(|check| check.get("severity").and_then(Value::as_str) != Some("error"));

    json!({
        "ok": compatible,
        "status": if compatible { "compatible" } else { "incompatible" },
        "local": {
            "version": local_version,
            "control_api_version": local_control,
            "peer_protocol_version": local_peer,
            "features": local_features,
        },
        "remote": {
            "version": peer.version,
            "control_api_version": peer.control_api_version,
            "peer_protocol_version": peer.peer_protocol_version,
            "features": peer.features,
            "common_features": common_features,
            "missing_features": missing_features,
            "os": peer.os,
            "arch": peer.arch,
        },
        "checks": checks,
        "next_action": next_action(peer, compatible),
    })
}

fn control_api_check(local: u16, remote: Option<u16>) -> Value {
    match remote {
        Some(remote) if remote <= local => json!({
            "name": "control_api_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "info",
        }),
        Some(remote) => json!({
            "name": "control_api_version",
            "ok": false,
            "local": local,
            "remote": remote,
            "severity": "error",
            "message": "saved peer control API is newer than this binary supports",
        }),
        None => json!({
            "name": "control_api_version",
            "ok": false,
            "local": local,
            "remote": Value::Null,
            "severity": "error",
            "message": "saved peer record has no control API version; refresh the descriptor",
        }),
    }
}

fn peer_protocol_check(local: u16, remote: Option<u16>) -> Value {
    match remote {
        Some(remote) if remote == local => json!({
            "name": "peer_protocol_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "info",
        }),
        Some(remote) if remote > local => json!({
            "name": "peer_protocol_version",
            "ok": false,
            "local": local,
            "remote": remote,
            "severity": "error",
            "message": "saved peer data protocol is newer than this binary supports",
        }),
        Some(remote) => json!({
            "name": "peer_protocol_version",
            "ok": false,
            "local": local,
            "remote": remote,
            "severity": "error",
            "message": "saved peer data protocol is older than this binary requires",
        }),
        None => json!({
            "name": "peer_protocol_version",
            "ok": false,
            "local": local,
            "remote": Value::Null,
            "severity": "error",
            "message": "saved peer record has no peer data protocol version; refresh the descriptor",
        }),
    }
}

fn feature_check(local: &[String], remote: &[String], missing: &[String]) -> Value {
    if missing.is_empty() {
        json!({
            "name": "features",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "info",
        })
    } else {
        json!({
            "name": "features",
            "ok": false,
            "local": local,
            "remote": remote,
            "severity": "error",
            "missing": missing,
            "message": "saved peer record is missing required data-plane feature flags",
        })
    }
}

fn binary_version_check(local: &str, remote: Option<&str>) -> Value {
    match remote.and_then(|remote| compare_dotted_versions(local, remote)) {
        Some(std::cmp::Ordering::Equal) => json!({
            "name": "binary_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "info",
        }),
        Some(std::cmp::Ordering::Greater) => json!({
            "name": "binary_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "warning",
            "message": "saved peer binary is older; refresh or bootstrap the peer when changing protocols",
        }),
        Some(std::cmp::Ordering::Less) => json!({
            "name": "binary_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "warning",
            "message": "saved peer binary is newer; consider upgrading the local binary",
        }),
        None => json!({
            "name": "binary_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "warning",
            "message": "binary version is not recorded or cannot be compared",
        }),
    }
}

fn next_action(peer: &config::PeerRecord, compatible: bool) -> &'static str {
    if !compatible {
        if peer
            .control_api_version
            .is_some_and(|remote| remote > node_daemon::control_api_version())
            || peer
                .peer_protocol_version
                .is_some_and(|remote| remote > node_daemon::peer_protocol_version())
        {
            "upgrade-local"
        } else {
            "peer-refresh"
        }
    } else if peer.version.as_deref().is_some_and(|remote| {
        compare_dotted_versions(env!("CARGO_PKG_VERSION"), remote)
            == Some(std::cmp::Ordering::Greater)
    }) {
        "peer-bootstrap --force"
    } else {
        "none"
    }
}

fn compare_dotted_versions(left: &str, right: &str) -> Option<std::cmp::Ordering> {
    let left = parse_dotted_version(left)?;
    let right = parse_dotted_version(right)?;
    Some(left.cmp(&right))
}

fn parse_dotted_version(value: &str) -> Option<Vec<u64>> {
    let core = value.split_once('-').map(|(core, _)| core).unwrap_or(value);
    let parts = core
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (!parts.is_empty()).then_some(parts)
}
