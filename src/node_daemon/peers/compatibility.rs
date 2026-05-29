use serde_json::{Value, json};

use crate::{
    cli, config, deploy,
    node_daemon::control_protocol,
    peer_transport,
    protocol_core::version::{compare_dotted_versions, protocol_compatibility_report},
};

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
    let protocol_report = protocol_compatibility_report(
        local_control,
        remote_control,
        local_peer,
        remote_peer,
        &local_features,
        &remote_features,
    );

    let mut checks = protocol_report.checks;
    checks.push(binary_version_check(local_version, remote_version));
    let compatible = checks
        .iter()
        .all(|check| check.get("severity").and_then(Value::as_str) != Some("error"));
    let next_action = version_next_action(
        remote_version,
        remote_control,
        remote_peer,
        local_control,
        local_peer,
        &protocol_report.missing_features,
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
            "common_features": protocol_report.common_features,
            "missing_features": protocol_report.missing_features,
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
    let protocol_report = protocol_compatibility_report(
        local_control,
        remote_control,
        local_peer,
        remote_peer,
        &local_features,
        &remote_features,
    );

    let mut checks = protocol_report.checks;
    checks.push(binary_version_check(local_version, remote_version));
    let compatible = checks
        .iter()
        .all(|check| check.get("severity").and_then(Value::as_str) != Some("error"));
    let next_action = version_next_action(
        remote_version,
        remote_control,
        remote_peer,
        local_control,
        local_peer,
        &protocol_report.missing_features,
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
            "common_features": protocol_report.common_features,
            "missing_features": protocol_report.missing_features,
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
