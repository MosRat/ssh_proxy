use serde_json::{Value, json};

use crate::{config, deploy};

use super::compatibility;

pub(super) fn build_peer_diff(
    alias: &str,
    config: &config::AppConfig,
    result: &deploy::RemoteDescriptorResult,
) -> Value {
    let local_peer = config.peers.get(alias);
    let local_profile = config.profiles.get(alias);
    let local = local_peer_summary(alias, local_peer, local_profile);
    let remote = remote_descriptor_summary(result);
    let mut diffs = Vec::new();

    if local_peer.is_none() {
        diffs.push(json!({
            "field": "peer.recorded",
            "local": false,
            "remote": true,
            "action": "peer-refresh"
        }));
    }
    if local_profile.is_none() {
        diffs.push(json!({
            "field": "profile.recorded",
            "local": false,
            "remote": true,
            "action": "peer-refresh"
        }));
    }

    push_diff(
        &mut diffs,
        "peer.target",
        local
            .pointer("/peer/target")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("target").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.node_id",
        local
            .pointer("/peer/node_id")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("node_id").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.node_name",
        local
            .pointer("/peer/node_name")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("node_name").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.service_instance_id",
        local
            .pointer("/peer/service_instance_id")
            .cloned()
            .unwrap_or(Value::Null),
        remote
            .get("service_instance_id")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.version",
        local
            .pointer("/peer/version")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("version").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.remote_path",
        local
            .pointer("/peer/remote_path")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("remote_path").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.control_endpoint",
        local
            .pointer("/peer/control_endpoint")
            .cloned()
            .unwrap_or(Value::Null),
        remote
            .get("control_endpoint")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.transport",
        local
            .pointer("/peer/transport")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("transport").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.tls_transport",
        local
            .pointer("/peer/tls_transport")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("tls_transport").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.quic_transport",
        local
            .pointer("/peer/quic_transport")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("quic_transport").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.transport_protocols",
        local
            .pointer("/peer/transport_protocols")
            .cloned()
            .unwrap_or(Value::Null),
        remote
            .get("transport_protocols")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "auth.token",
        local.pointer("/auth/token").cloned().unwrap_or(Value::Null),
        remote
            .pointer("/auth/token")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-rotate-token",
    );
    push_diff(
        &mut diffs,
        "auth.token_scope",
        local
            .pointer("/auth/token_scope")
            .cloned()
            .unwrap_or(Value::Null),
        remote
            .pointer("/auth/token_scope")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-rotate-token",
    );
    push_diff(
        &mut diffs,
        "auth.token_generation",
        local
            .pointer("/auth/token_generation")
            .cloned()
            .unwrap_or(Value::Null),
        remote
            .pointer("/auth/token_generation")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-rotate-token",
    );
    push_diff(
        &mut diffs,
        "auth.tls_server_cert_fingerprint",
        local
            .pointer("/auth/tls_server_cert_fingerprint")
            .cloned()
            .unwrap_or(Value::Null),
        remote
            .pointer("/auth/tls_server_cert_fingerprint")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "auth.tls_client_ca_fingerprint",
        local
            .pointer("/auth/tls_client_ca_fingerprint")
            .cloned()
            .unwrap_or(Value::Null),
        remote
            .pointer("/auth/tls_client_ca_fingerprint")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "profile.remote_control",
        local
            .pointer("/profile/remote_control")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("remote_control").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "profile.remote_tcp",
        local
            .pointer("/profile/remote_tcp")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("transport").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "profile.remote_tls",
        local
            .pointer("/profile/remote_tls")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("tls_transport").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "profile.remote_quic",
        local
            .pointer("/profile/remote_quic")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("quic_transport").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "profile.remote_token",
        local
            .pointer("/profile/remote_token")
            .cloned()
            .unwrap_or(Value::Null),
        remote
            .pointer("/auth/token")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-rotate-token",
    );

    let changed = !diffs.is_empty();
    let next_action = next_peer_diff_action(&diffs);
    json!({
        "ok": true,
        "kind": "peer_diff",
        "alias": alias,
        "target": result.target,
        "changed": changed,
        "local": local,
        "remote": remote,
        "diffs": diffs,
        "next_action": next_action,
    })
}

pub(super) fn build_peer_reconcile(
    alias: &str,
    config: &config::AppConfig,
    result: &deploy::RemoteDescriptorResult,
) -> Value {
    let local_peer = config.peers.get(alias);
    let local_profile = config.profiles.get(alias);
    let local = local_peer_summary(alias, local_peer, local_profile);
    let remote = remote_descriptor_summary(result);
    let diff = build_peer_diff(alias, config, result);
    let version = compatibility::build_peer_version_check(alias, result);
    let mut issues = Vec::new();

    if local_peer.is_none() || local_profile.is_none() {
        issues.push(json!({
            "code": "missing_local_record",
            "severity": "warning",
            "message": "remote daemon descriptor exists but the local peer/profile record is incomplete",
            "repair_command": format!("ssh_proxy node control peer-refresh {target} --alias {alias}", target = result.target),
        }));
    }

    let local_node_id = local
        .pointer("/peer/node_id")
        .cloned()
        .unwrap_or(Value::Null);
    let remote_node_id = remote.get("node_id").cloned().unwrap_or(Value::Null);
    if !local_node_id.is_null() && !remote_node_id.is_null() && local_node_id != remote_node_id {
        issues.push(json!({
            "code": "stale_remote_record",
            "severity": "warning",
            "message": "local peer identity points at a different remote node id",
            "repair_command": format!("ssh_proxy node control peer-refresh {target} --alias {alias}", target = result.target),
        }));
    }

    if version.get("status").and_then(Value::as_str) != Some("compatible") {
        let compatible = version
            .get("compatible")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        issues.push(json!({
            "code": "version_mismatch",
            "severity": if compatible { "warning" } else { "error" },
            "message": if compatible {
                "remote version differs but advertised protocols/features still allow fallback"
            } else {
                "remote version or feature set is incompatible with this binary"
            },
            "fallback_when_compatible": compatible,
            "repair_command": version.get("next_action").cloned().unwrap_or(Value::String("peer-bootstrap --force".to_string())),
        }));
    }

    let local_token = local.pointer("/auth/token").cloned().unwrap_or(Value::Null);
    let remote_token = remote
        .pointer("/auth/token")
        .cloned()
        .unwrap_or(Value::Null);
    let local_generation = local
        .pointer("/auth/token_generation")
        .cloned()
        .unwrap_or(Value::Null);
    let remote_generation = remote
        .pointer("/auth/token_generation")
        .cloned()
        .unwrap_or(Value::Null);
    if local_token != remote_token
        || (!local_generation.is_null()
            && !remote_generation.is_null()
            && local_generation != remote_generation)
    {
        issues.push(json!({
            "code": "token_mismatch",
            "severity": "warning",
            "message": "local record and remote descriptor disagree about token presence or token generation",
            "repair_command": format!("ssh_proxy node control peer-rotate-token {target} --alias {alias}", target = result.target),
        }));
    }

    let cert_pairs = [
        (
            "tls_server_cert_fingerprint",
            local.pointer("/auth/tls_server_cert_fingerprint"),
            remote.pointer("/auth/tls_server_cert_fingerprint"),
        ),
        (
            "tls_client_ca_fingerprint",
            local.pointer("/auth/tls_client_ca_fingerprint"),
            remote.pointer("/auth/tls_client_ca_fingerprint"),
        ),
    ];
    for (field, local_value, remote_value) in cert_pairs {
        if let (Some(local_value), Some(remote_value)) = (local_value, remote_value)
            && !local_value.is_null()
            && !remote_value.is_null()
            && local_value != remote_value
        {
            issues.push(json!({
                "code": "certificate_mismatch",
                "field": field,
                "severity": "warning",
                "message": "local certificate fingerprint differs from the remote descriptor",
                "repair_command": format!("ssh_proxy node control peer-refresh {target} --alias {alias}", target = result.target),
            }));
        }
    }

    for field in ["service_instance_id", "os_user", "data_dir"] {
        let local_value = local
            .pointer(&format!("/peer/{field}"))
            .cloned()
            .unwrap_or(Value::Null);
        let remote_value = remote.get(field).cloned().unwrap_or(Value::Null);
        if !local_value.is_null() && !remote_value.is_null() && local_value != remote_value {
            issues.push(json!({
                "code": "ownership_mismatch",
                "field": field,
                "severity": "warning",
                "message": "local peer record points at a different daemon owner or service instance",
                "repair_command": format!("ssh_proxy node control peer-refresh {target} --alias {alias}", target = result.target),
            }));
        }
    }

    let adoption_plan = if local_peer.is_none() || local_profile.is_none() {
        json!({
            "needed": true,
            "mode": "adopt-existing-daemon",
            "command": format!("ssh_proxy node control peer-refresh {target} --alias {alias}", target = result.target),
            "will_replace_auth_material": false,
            "note": "dry-run only; refresh/adoption must be invoked explicitly"
        })
    } else {
        json!({
            "needed": false,
            "mode": "none",
            "will_replace_auth_material": false
        })
    };

    json!({
        "ok": true,
        "kind": "peer_reconcile",
        "alias": alias,
        "target": result.target,
        "changed": false,
        "dry_run": true,
        "explicit_repair_required": true,
        "local": local,
        "remote": remote,
        "diff": diff,
        "version": version,
        "issues": issues,
        "adoption_plan": adoption_plan,
        "repair_commands": issues
            .iter()
            .filter_map(|issue| issue.get("repair_command").cloned())
            .collect::<Vec<_>>(),
    })
}

fn local_peer_summary(
    alias: &str,
    peer: Option<&config::PeerRecord>,
    profile: Option<&config::ProxyProfile>,
) -> Value {
    json!({
        "alias": alias,
        "peer": {
            "recorded": peer.is_some(),
            "target": peer.and_then(|peer| peer.target.clone()),
            "node_id": peer.and_then(|peer| peer.node_id.clone()),
            "node_name": peer.and_then(|peer| peer.node_name.clone()),
            "service_instance_id": peer.and_then(|peer| peer.service_instance_id.clone()),
            "trust": peer.and_then(|peer| peer.trust.clone()),
            "remote_path": peer.and_then(|peer| peer.remote_path.clone()),
            "control_endpoint": peer.and_then(|peer| peer.control_endpoint.clone()),
            "transport": peer.and_then(|peer| peer.transport).map(|addr| addr.to_string()),
            "tls_transport": peer.and_then(|peer| peer.tls_transport).map(|addr| addr.to_string()),
            "quic_transport": peer.and_then(|peer| peer.quic_transport).map(|addr| addr.to_string()),
            "transport_protocols": peer.map(config::PeerRecord::known_transport_protocols).unwrap_or_default(),
            "version": peer.and_then(|peer| peer.version.clone()),
            "control_api_version": peer.and_then(|peer| peer.control_api_version),
            "peer_protocol_version": peer.and_then(|peer| peer.peer_protocol_version),
            "features": peer.map(|peer| peer.features.clone()).unwrap_or_default(),
            "os_user": peer.and_then(|peer| peer.os_user.clone()),
            "data_dir": peer.and_then(|peer| peer.data_dir.clone()),
            "last_seen_unix": peer.and_then(|peer| peer.last_seen_unix),
        },
        "profile": {
            "recorded": profile.is_some(),
            "target": profile.and_then(|profile| profile.target.clone()),
            "remote_path": profile.and_then(|profile| profile.remote_path.clone()),
            "remote_control": profile.and_then(|profile| profile.remote_control).map(|addr| addr.to_string()),
            "remote_tcp": profile.and_then(|profile| profile.remote_tcp).map(|addr| addr.to_string()),
            "remote_tls": profile.and_then(|profile| profile.remote_tls).map(|addr| addr.to_string()),
            "remote_quic": profile.and_then(|profile| profile.remote_quic).map(|addr| addr.to_string()),
            "remote_transport": profile.and_then(|profile| profile.remote_transport.clone()),
            "remote_token": profile.and_then(|profile| profile.remote_token.as_ref()).is_some(),
        },
        "auth": {
            "token": peer.and_then(|peer| peer.token.as_ref()).is_some(),
            "token_metadata": peer.and_then(|peer| peer.token_metadata.clone()),
            "token_scope": peer
                .and_then(|peer| peer.token_metadata.as_ref())
                .map(|metadata| metadata.scope.clone()),
            "token_generation": peer
                .and_then(|peer| peer.token_metadata.as_ref())
                .map(|metadata| metadata.generation),
            "tls_server_cert_fingerprint": peer.and_then(|peer| peer.tls_server_cert_fingerprint.clone()),
            "tls_client_ca_fingerprint": peer.and_then(|peer| peer.tls_client_ca_fingerprint.clone()),
        }
    })
}

fn remote_descriptor_summary(result: &deploy::RemoteDescriptorResult) -> Value {
    let descriptor = &result.descriptor;
    let control_endpoint = descriptor
        .pointer("/endpoints/control")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("tcp://{}", result.remote_control));
    let token_metadata = descriptor.pointer("/auth/token_metadata").cloned();
    let token_scope = token_metadata
        .as_ref()
        .and_then(|metadata| metadata.get("scope"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    json!({
        "target": result.target,
        "node_id": descriptor.get("node_id").cloned().unwrap_or(Value::Null),
        "node_name": descriptor.get("node_name").cloned().unwrap_or(Value::Null),
        "service_instance_id": descriptor.get("service_instance_id").cloned().unwrap_or(Value::Null),
        "version": descriptor.get("version").cloned().unwrap_or(Value::Null),
        "control_api_version": descriptor.get("control_api_version").cloned().unwrap_or(Value::Null),
        "peer_protocol_version": descriptor.get("peer_protocol_version").cloned().unwrap_or(Value::Null),
        "features": descriptor.get("features").cloned().unwrap_or(Value::Array(Vec::new())),
        "os_user": descriptor.get("os_user").cloned().unwrap_or(Value::Null),
        "data_dir": descriptor.get("data_dir").cloned().unwrap_or(Value::Null),
        "remote_path": result.remote_path,
        "control_endpoint": control_endpoint,
        "remote_control": result.remote_control.to_string(),
        "transport": result.remote_tcp.to_string(),
        "tls_transport": result.remote_tls_transport.map(|addr| addr.to_string()),
        "quic_transport": result.remote_quic_transport.map(|addr| addr.to_string()),
        "transport_protocols": descriptor_protocols(descriptor).unwrap_or_else(|| {
            let mut protocols = Vec::new();
            if result.remote_quic_transport.is_some() {
                protocols.push("quic".to_string());
            }
            if result.remote_tls_transport.is_some() {
                protocols.push("tls-tcp".to_string());
            }
            protocols.push("plain-tcp".to_string());
            protocols
        }),
        "auth": {
            "token": descriptor
                .pointer("/auth/control_token")
                .and_then(Value::as_bool)
                .unwrap_or(result.remote_token.is_some()),
            "token_metadata": token_metadata,
            "token_scope": token_scope,
            "token_generation": descriptor
                .pointer("/auth/token_generation")
                .cloned()
                .or_else(|| descriptor.pointer("/auth/token_metadata/generation").cloned())
                .unwrap_or(Value::Null),
            "tls_server_cert_fingerprint": descriptor
                .pointer("/auth/tls_server_cert_fingerprint")
                .cloned()
                .unwrap_or(Value::Null),
            "tls_client_ca_fingerprint": descriptor
                .pointer("/auth/tls_client_ca_fingerprint")
                .cloned()
                .unwrap_or(Value::Null),
        }
    })
}

fn descriptor_protocols(descriptor: &Value) -> Option<Vec<String>> {
    let protocols = descriptor
        .get("transport_protocols")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    (!protocols.is_empty()).then_some(protocols)
}

fn push_diff(diffs: &mut Vec<Value>, field: &str, local: Value, remote: Value, action: &str) {
    if local != remote {
        diffs.push(json!({
            "field": field,
            "local": local,
            "remote": remote,
            "action": action
        }));
    }
}

fn next_peer_diff_action(diffs: &[Value]) -> &'static str {
    if diffs.is_empty() {
        return "none";
    }
    let has_refresh = diffs
        .iter()
        .any(|diff| diff.get("action").and_then(Value::as_str) == Some("peer-refresh"));
    if has_refresh {
        "peer-refresh"
    } else {
        "peer-rotate-token"
    }
}
