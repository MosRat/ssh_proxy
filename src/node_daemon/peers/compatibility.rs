use serde_json::{Value, json};

use crate::{cli, config, deploy, node_daemon::control_protocol, peer_transport};

pub(super) fn build_peer_version_check(
    alias: &str,
    result: &deploy::RemoteDescriptorResult,
) -> Value {
    let descriptor = &result.descriptor;
    let local_version = env!("CARGO_PKG_VERSION");
    let remote_version = descriptor.get("version").and_then(Value::as_str);
    let local_control = control_protocol::NODE_CONTROL_VERSION;
    let remote_control = descriptor.get("control_api_version").and_then(value_to_u16);
    let local_peer = peer_transport::PEER_VERSION;
    let remote_peer = descriptor
        .get("peer_protocol_version")
        .and_then(value_to_u16);
    let local_features = peer_transport::default_features();
    let remote_features = string_array_field(descriptor, "features");
    let missing_features = local_features
        .iter()
        .filter(|feature| !remote_features.contains(*feature))
        .cloned()
        .collect::<Vec<_>>();
    let common_features = local_features
        .iter()
        .filter(|feature| remote_features.contains(*feature))
        .cloned()
        .collect::<Vec<_>>();

    let checks = vec![
        control_api_check(local_control, remote_control),
        peer_protocol_check(local_peer, remote_peer),
        feature_check(&local_features, &remote_features, &missing_features),
        binary_version_check(local_version, remote_version),
    ];
    let compatible = checks
        .iter()
        .all(|check| check.get("severity").and_then(Value::as_str) != Some("error"));
    let next_action = version_next_action(
        remote_version,
        remote_control,
        remote_peer,
        local_control,
        local_peer,
        &missing_features,
    );
    let status = version_status(compatible, local_version, remote_version, next_action);

    json!({
        "ok": true,
        "kind": "peer_version_check",
        "alias": alias,
        "target": result.target,
        "compatible": compatible,
        "status": status,
        "local": {
            "version": local_version,
            "control_api_version": local_control,
            "peer_protocol_version": local_peer,
            "features": local_features,
        },
        "remote": {
            "version": remote_version,
            "control_api_version": remote_control,
            "peer_protocol_version": remote_peer,
            "features": remote_features,
            "common_features": common_features,
            "missing_features": missing_features,
            "os": descriptor.get("os").cloned().unwrap_or(Value::Null),
            "arch": descriptor.get("arch").cloned().unwrap_or(Value::Null),
        },
        "checks": checks,
        "next_action": next_action,
    })
}

pub(super) fn build_saved_peer_version_check(alias: &str, peer: &config::PeerRecord) -> Value {
    let local_version = env!("CARGO_PKG_VERSION");
    let remote_version = peer.version.as_deref();
    let local_control = control_protocol::NODE_CONTROL_VERSION;
    let remote_control = peer.control_api_version;
    let local_peer = peer_transport::PEER_VERSION;
    let remote_peer = peer.peer_protocol_version;
    let local_features = peer_transport::default_features();
    let remote_features = peer.features.clone();
    let missing_features = local_features
        .iter()
        .filter(|feature| !remote_features.contains(*feature))
        .cloned()
        .collect::<Vec<_>>();
    let common_features = local_features
        .iter()
        .filter(|feature| remote_features.contains(*feature))
        .cloned()
        .collect::<Vec<_>>();

    let checks = vec![
        control_api_check(local_control, remote_control),
        peer_protocol_check(local_peer, remote_peer),
        feature_check(&local_features, &remote_features, &missing_features),
        binary_version_check(local_version, remote_version),
    ];
    let compatible = checks
        .iter()
        .all(|check| check.get("severity").and_then(Value::as_str) != Some("error"));
    let next_action = version_next_action(
        remote_version,
        remote_control,
        remote_peer,
        local_control,
        local_peer,
        &missing_features,
    );
    let status = version_status(compatible, local_version, remote_version, next_action);

    json!({
        "kind": "saved_peer_version_check",
        "alias": alias,
        "recorded": true,
        "fresh": false,
        "compatible": compatible,
        "status": status,
        "local": {
            "version": local_version,
            "control_api_version": local_control,
            "peer_protocol_version": local_peer,
            "features": local_features,
        },
        "remote": {
            "version": remote_version,
            "control_api_version": remote_control,
            "peer_protocol_version": remote_peer,
            "features": remote_features,
            "common_features": common_features,
            "missing_features": missing_features,
            "os": peer.os,
            "arch": peer.arch,
        },
        "checks": checks,
        "next_action": next_action,
    })
}

pub(super) fn attach_saved_peer_compatibility(
    plan: &mut Value,
    args: &cli::RouteArgs,
    config: &config::AppConfig,
) {
    let compatibility = config
        .peers
        .get(&args.target)
        .map(|peer| build_saved_peer_version_check(&args.target, peer))
        .unwrap_or_else(|| {
            json!({
                "kind": "saved_peer_version_check",
                "alias": args.target,
                "recorded": false,
                "compatible": false,
                "status": "unrecorded",
                "checks": [],
                "next_action": "peer-bootstrap",
                "message": "peer is not recorded locally; route start will try descriptor adoption then SSH bootstrap"
            })
        });
    if let Value::Object(object) = plan {
        object.insert("peer_compatibility".to_string(), compatibility);
    }
}

fn control_api_check(local: u16, remote: Option<u16>) -> Value {
    match remote {
        Some(remote) if remote <= local => json!({
            "name": "control_api_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "info",
            "message": "remote control API is supported"
        }),
        Some(remote) => json!({
            "name": "control_api_version",
            "ok": false,
            "local": local,
            "remote": remote,
            "severity": "error",
            "message": "remote control API is newer than this binary supports"
        }),
        None => json!({
            "name": "control_api_version",
            "ok": false,
            "local": local,
            "remote": Value::Null,
            "severity": "error",
            "message": "remote descriptor does not advertise a control API version"
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
            "message": "remote peer data protocol matches"
        }),
        Some(remote) if remote > local => json!({
            "name": "peer_protocol_version",
            "ok": false,
            "local": local,
            "remote": remote,
            "severity": "error",
            "message": "remote peer data protocol is newer than this binary supports"
        }),
        Some(remote) => json!({
            "name": "peer_protocol_version",
            "ok": false,
            "local": local,
            "remote": remote,
            "severity": "error",
            "message": "remote peer data protocol is older than this binary requires"
        }),
        None => json!({
            "name": "peer_protocol_version",
            "ok": false,
            "local": local,
            "remote": Value::Null,
            "severity": "error",
            "message": "remote descriptor does not advertise a peer data protocol version"
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
            "message": "remote advertises all locally required data-plane features"
        })
    } else {
        json!({
            "name": "features",
            "ok": false,
            "local": local,
            "remote": remote,
            "severity": "error",
            "message": "remote is missing required data-plane features",
            "missing": missing,
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
            "message": "local and remote binaries report the same package version"
        }),
        Some(std::cmp::Ordering::Greater) => json!({
            "name": "binary_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "warning",
            "message": "remote binary is older; bootstrap with --force to align versions"
        }),
        Some(std::cmp::Ordering::Less) => json!({
            "name": "binary_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "warning",
            "message": "remote binary is newer; consider upgrading the local binary"
        }),
        None => json!({
            "name": "binary_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "warning",
            "message": "binary version could not be compared"
        }),
    }
}

fn version_next_action(
    remote_version: Option<&str>,
    remote_control: Option<u16>,
    remote_peer: Option<u16>,
    local_control: u16,
    local_peer: u16,
    missing_features: &[String],
) -> &'static str {
    if remote_control.is_some_and(|remote| remote > local_control)
        || remote_peer.is_some_and(|remote| remote > local_peer)
    {
        return "upgrade-local";
    }
    if remote_control.is_none()
        || remote_peer.is_none()
        || remote_peer.is_some_and(|remote| remote < local_peer)
        || !missing_features.is_empty()
    {
        return "peer-bootstrap --force";
    }
    match remote_version
        .and_then(|remote| compare_dotted_versions(env!("CARGO_PKG_VERSION"), remote))
    {
        Some(std::cmp::Ordering::Greater) => "peer-bootstrap --force",
        Some(std::cmp::Ordering::Less) => "upgrade-local",
        _ => "none",
    }
}

fn version_status(
    compatible: bool,
    local_version: &str,
    remote_version: Option<&str>,
    next_action: &str,
) -> &'static str {
    if !compatible {
        return "incompatible";
    }
    match remote_version.and_then(|remote| compare_dotted_versions(local_version, remote)) {
        Some(std::cmp::Ordering::Equal) => "compatible",
        Some(std::cmp::Ordering::Greater) if next_action == "peer-bootstrap --force" => {
            "compatible-upgrade-remote"
        }
        Some(std::cmp::Ordering::Less) if next_action == "upgrade-local" => {
            "compatible-upgrade-local"
        }
        _ => "compatible-version-unknown",
    }
}

fn value_to_u16(value: &Value) -> Option<u16> {
    value.as_u64().and_then(|value| u16::try_from(value).ok())
}

fn string_array_field(value: &Value, field: &str) -> Vec<String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
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
