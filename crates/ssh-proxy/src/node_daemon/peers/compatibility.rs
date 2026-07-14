use serde_json::Value;
use ssh_proxy_service::{
    PeerCompatibilityInput, PeerVersionCheckInput, peer_version_check_report,
    unrecorded_peer_version_check_report,
};

use crate::{cli, config, deploy};

pub(super) fn build_peer_version_check(
    alias: &str,
    result: &deploy::RemoteDescriptorResult,
) -> Value {
    let descriptor = &result.descriptor;
    peer_version_check_report(PeerVersionCheckInput {
        ok: Some(true),
        kind: "peer_version_check".to_string(),
        alias: alias.to_string(),
        target: Some(result.target.clone()),
        recorded: None,
        fresh: None,
        compatibility: compatibility_input(
            descriptor.get("version").and_then(Value::as_str),
            descriptor.get("control_api_version").and_then(value_to_u16),
            descriptor
                .get("peer_protocol_version")
                .and_then(value_to_u16),
            string_array_field(descriptor, "features"),
            descriptor
                .get("os")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            descriptor
                .get("arch")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        ),
    })
}

pub(super) fn build_saved_peer_version_check(alias: &str, peer: &config::PeerRecord) -> Value {
    peer_version_check_report(PeerVersionCheckInput {
        ok: None,
        kind: "saved_peer_version_check".to_string(),
        alias: alias.to_string(),
        target: None,
        recorded: Some(true),
        fresh: Some(false),
        compatibility: compatibility_input(
            peer.version.as_deref(),
            peer.control_api_version,
            peer.peer_protocol_version,
            peer.features.clone(),
            peer.os.clone(),
            peer.arch.clone(),
        ),
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
        .unwrap_or_else(|| unrecorded_peer_version_check_report(&args.target));
    if let Value::Object(object) = plan {
        object.insert("peer_compatibility".to_string(), compatibility);
    }
}

fn compatibility_input(
    remote_version: Option<&str>,
    remote_control: Option<u16>,
    remote_peer: Option<u16>,
    remote_features: Vec<String>,
    remote_os: Option<String>,
    remote_arch: Option<String>,
) -> PeerCompatibilityInput {
    PeerCompatibilityInput {
        local_version: env!("CARGO_PKG_VERSION").to_string(),
        local_control_api_version: crate::node_daemon::control_api_version(),
        local_peer_protocol_version: crate::node_daemon::peer_protocol_version(),
        local_features: crate::node_daemon::peer_protocol_features(),
        remote_version: remote_version.map(ToOwned::to_owned),
        remote_control_api_version: remote_control,
        remote_peer_protocol_version: remote_peer,
        remote_features,
        remote_os,
        remote_arch,
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
