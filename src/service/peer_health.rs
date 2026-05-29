use serde_json::{Value, json};

use crate::{
    config, node_daemon,
    protocol_core::version::{compare_dotted_versions, protocol_compatibility_report},
};

pub(super) fn compatibility(peer: &config::PeerRecord) -> Value {
    let local_version = env!("CARGO_PKG_VERSION");
    let local_control = node_daemon::control_api_version();
    let local_peer = node_daemon::peer_protocol_version();
    let local_features = node_daemon::peer_protocol_features();
    let protocol_report = protocol_compatibility_report(
        local_control,
        peer.control_api_version,
        local_peer,
        peer.peer_protocol_version,
        &local_features,
        &peer.features,
    );
    let mut checks = protocol_report.checks;
    checks.push(binary_version_check(local_version, peer.version.as_deref()));
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
            "common_features": protocol_report.common_features,
            "missing_features": protocol_report.missing_features,
            "os": peer.os,
            "arch": peer.arch,
        },
        "checks": checks,
        "next_action": next_action(peer, compatible),
    })
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
