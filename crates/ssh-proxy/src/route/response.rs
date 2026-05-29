use std::net::SocketAddr;

use serde_json::{Value, json};

use crate::{cli, ssh_client};

use super::policy::pool_policy_name;
use super::transport::{
    direct_transport_policy, direct_transport_policy_reason, remote_transport_name,
    ssh_data_plane_reason, ssh_mode_name, ssh_mode_reason, tls_peer_auth_mode,
};

pub(crate) fn local_uses_remote_plan(
    args: &cli::RouteArgs,
    id: &str,
    forward: &cli::NodeForwardArgs,
) -> Value {
    let mut plan = json!({
        "route_id": id,
        "direction": "local-uses-remote",
        "owner": "local",
        "mode": "local-forward",
        "listener": {
            "owner": "local",
            "listen": forward.listen.to_string(),
            "tcp_target": forward.tcp_target.as_ref().map(ToString::to_string),
        },
        "egress": {
            "peer": args.target,
            "side": "remote",
            "upstream_proxy": forward.egress_proxy.clone(),
        },
        "transport_candidates": transport_candidates(forward),
        "selected_transport": remote_transport_name(forward.remote_transport),
        "transport_selection_source": forward
            .transport_selection_source
            .as_deref()
            .unwrap_or("unknown"),
        "transport_selection_reason": forward
            .transport_selection_reason
            .as_deref()
            .unwrap_or("unknown"),
        "direct_transport_policy": direct_transport_policy(forward.remote_transport),
        "direct_transport_policy_reason": direct_transport_policy_reason(forward.remote_transport),
        "tls_peer_auth_mode": tls_peer_auth_mode(
            forward.remote_transport,
            forward.remote_client_cert.as_ref(),
            forward.remote_client_key.as_ref(),
        ),
        "ssh_mode": ssh_mode_name(forward.remote_transport),
        "ssh_mode_reason": ssh_mode_reason(forward.remote_transport),
        "ssh_data_plane_reason": ssh_data_plane_reason(
            forward.remote_transport,
            forward.transport_selection_source.as_deref(),
        ),
        "ssh_session_pool_size": if matches!(forward.remote_transport, cli::RemoteTransport::SshNative) {
            json!(forward.ssh_session_pool_size.unwrap_or(1))
        } else {
            Value::Null
        },
        "ssh_session_pool_source": if matches!(forward.remote_transport, cli::RemoteTransport::SshNative) {
            json!(forward.ssh_session_pool_source.as_deref().unwrap_or("unknown"))
        } else {
            Value::Null
        },
        "ssh_session_pool_reason": if matches!(forward.remote_transport, cli::RemoteTransport::SshNative) {
            json!(forward.ssh_session_pool_reason.as_deref().unwrap_or("unknown"))
        } else {
            Value::Null
        },
        "ssh_session_pool_warning": if matches!(forward.remote_transport, cli::RemoteTransport::SshNative) {
            json!(forward.ssh_session_pool_warning.as_deref())
        } else {
            Value::Null
        },
        "topology": topology_hint(args, forward),
        "runtime": route_runtime_plan(
            forward.reconnect_delay_secs,
            forward.reconnect_max_delay_secs,
            forward.connect_timeout_secs,
            forward.transport_pool_size,
            forward.transport_pool_source.as_deref(),
            forward.transport_pool_reason.as_deref(),
            forward.pool_policy.as_deref(),
            forward.workload_hint.map(pool_policy_name),
            forward.no_reconnect,
        ),
        "fallback_reason": Value::Null,
        "next_action": "none",
        "persist": !args.volatile,
    });
    refresh_decision_chain(&mut plan);
    plan
}

pub(crate) fn remote_uses_local_reverse_link_plan(
    args: &cli::RouteArgs,
    id: &str,
    reverse: &cli::NodeReverseArgs,
    fallback_reason: Option<&str>,
) -> Value {
    let mut plan = json!({
        "route_id": id,
        "direction": "remote-uses-local",
        "owner": "local",
        "mode": "reverse-link",
        "listener": {
            "owner": "remote",
            "listen": reverse.remote_listen.to_string(),
            "tcp_target": reverse.tcp_target.as_ref().map(ToString::to_string),
        },
        "egress": {
            "peer": "local",
            "side": "local",
            "upstream_proxy": reverse.egress_proxy.clone(),
        },
        "transport_candidates": ["ssh-reverse-link"],
        "selected_transport": "ssh-reverse-link",
        "runtime": route_runtime_plan(
            reverse.reconnect_delay_secs,
            reverse.reconnect_max_delay_secs,
            reverse.connect_timeout_secs,
            1,
            reverse.transport_pool_source.as_deref(),
            reverse.transport_pool_reason.as_deref(),
            Some("large"),
            Some("large"),
            reverse.no_reconnect,
        ),
        "fallback_reason": fallback_reason,
        "next_action": if fallback_reason.is_some() {
            "set --local-peer <reachable-ip:port> for direct mode"
        } else {
            "none"
        },
        "persist": !args.volatile,
    });
    refresh_decision_chain(&mut plan);
    plan
}

pub(crate) fn remote_uses_local_direct_plan(
    args: &cli::RouteArgs,
    id: &str,
    forward: &cli::NodeForwardArgs,
    local_peer: SocketAddr,
) -> Value {
    let mut plan = json!({
        "route_id": id,
        "direction": "remote-uses-local",
        "owner": "remote",
        "mode": "direct",
        "listener": {
            "owner": "remote",
            "listen": forward.listen.to_string(),
            "tcp_target": forward.tcp_target.as_ref().map(ToString::to_string),
        },
        "egress": {
            "peer": "local",
            "side": "local",
            "reachable_peer": local_peer.to_string(),
            "upstream_proxy": forward.egress_proxy.clone(),
        },
        "transport_candidates": transport_candidates(forward),
        "selected_transport": remote_transport_name(forward.remote_transport),
        "transport_selection_source": forward
            .transport_selection_source
            .as_deref()
            .unwrap_or("unknown"),
        "transport_selection_reason": forward
            .transport_selection_reason
            .as_deref()
            .unwrap_or("unknown"),
        "direct_transport_policy": direct_transport_policy(forward.remote_transport),
        "direct_transport_policy_reason": direct_transport_policy_reason(forward.remote_transport),
        "tls_peer_auth_mode": tls_peer_auth_mode(
            forward.remote_transport,
            forward.remote_client_cert.as_ref(),
            forward.remote_client_key.as_ref(),
        ),
        "ssh_mode": ssh_mode_name(forward.remote_transport),
        "ssh_mode_reason": ssh_mode_reason(forward.remote_transport),
        "ssh_data_plane_reason": ssh_data_plane_reason(
            forward.remote_transport,
            forward.transport_selection_source.as_deref(),
        ),
        "ssh_session_pool_size": if matches!(forward.remote_transport, cli::RemoteTransport::SshNative) {
            json!(forward.ssh_session_pool_size.unwrap_or(1))
        } else {
            Value::Null
        },
        "ssh_session_pool_source": if matches!(forward.remote_transport, cli::RemoteTransport::SshNative) {
            json!(forward.ssh_session_pool_source.as_deref().unwrap_or("unknown"))
        } else {
            Value::Null
        },
        "ssh_session_pool_reason": if matches!(forward.remote_transport, cli::RemoteTransport::SshNative) {
            json!(forward.ssh_session_pool_reason.as_deref().unwrap_or("unknown"))
        } else {
            Value::Null
        },
        "ssh_session_pool_warning": if matches!(forward.remote_transport, cli::RemoteTransport::SshNative) {
            json!(forward.ssh_session_pool_warning.as_deref())
        } else {
            Value::Null
        },
        "topology": topology_hint(args, forward),
        "runtime": route_runtime_plan(
            forward.reconnect_delay_secs,
            forward.reconnect_max_delay_secs,
            forward.connect_timeout_secs,
            forward.transport_pool_size,
            forward.transport_pool_source.as_deref(),
            forward.transport_pool_reason.as_deref(),
            forward.pool_policy.as_deref(),
            forward.workload_hint.map(pool_policy_name),
            forward.no_reconnect,
        ),
        "fallback_reason": Value::Null,
        "next_action": "none",
        "persist": !args.volatile,
    });
    refresh_decision_chain(&mut plan);
    plan
}

pub(super) fn candidate_failures(results: &[Value]) -> Vec<Value> {
    results
        .iter()
        .filter(|result| is_direct_probe_protocol(result["protocol"].as_str()))
        .filter(|result| result["reachable"] == false)
        .cloned()
        .collect()
}

pub(super) fn is_direct_probe_protocol(protocol: Option<&str>) -> bool {
    matches!(protocol, Some("quic" | "tls-tcp" | "plain-tcp"))
}

fn route_runtime_plan(
    reconnect_delay_secs: u64,
    reconnect_max_delay_secs: u64,
    connect_timeout_secs: u64,
    transport_pool_size: usize,
    transport_pool_source: Option<&str>,
    transport_pool_reason: Option<&str>,
    pool_policy: Option<&str>,
    workload_hint: Option<&str>,
    no_reconnect: bool,
) -> Value {
    json!({
        "reconnect_delay_secs": reconnect_delay_secs,
        "reconnect_max_delay_secs": reconnect_max_delay_secs,
        "connect_timeout_secs": connect_timeout_secs,
        "transport_pool_size": transport_pool_size,
        "transport_pool_source": transport_pool_source.unwrap_or("implicit").to_string(),
        "transport_pool_reason": transport_pool_reason.unwrap_or("implicit single-worker default").to_string(),
        "pool_policy": pool_policy.unwrap_or("large").to_string(),
        "workload_hint": workload_hint.unwrap_or("large").to_string(),
        "no_reconnect": no_reconnect,
    })
}

pub(super) fn refresh_decision_chain(plan: &mut Value) {
    let selected_transport = plan
        .get("selected_transport")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let selection_source = plan
        .get("transport_selection_source")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let selection_reason = plan
        .get("transport_selection_reason")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let fallback_reason = plan.get("fallback_reason").cloned().unwrap_or(Value::Null);
    let next_action = plan
        .get("next_action")
        .and_then(Value::as_str)
        .unwrap_or("none");
    let direct_transport_policy = plan
        .get("direct_transport_policy")
        .cloned()
        .unwrap_or(Value::Null);
    let direct_transport_policy_reason = plan
        .get("direct_transport_policy_reason")
        .cloned()
        .unwrap_or(Value::Null);
    let tls_peer_auth_mode = plan
        .get("tls_peer_auth_mode")
        .cloned()
        .unwrap_or(Value::Null);
    let runtime = plan.get("runtime");
    let topology = plan.get("topology");
    let preflight = plan.get("preflight");
    let topology_class = route_topology_class(preflight, topology);
    let decision_chain = json!({
        "preflight": {
            "kind": preflight.and_then(|value| value.get("kind")).cloned().unwrap_or(Value::Null),
            "recommended_fallback": preflight.and_then(|value| value.get("recommended_fallback")).cloned().unwrap_or(Value::Null),
            "selected_reason": preflight.and_then(|value| value.get("selected_reason")).cloned().unwrap_or(Value::Null),
            "repair_hint": preflight.and_then(|value| value.get("repair_hint")).cloned().unwrap_or(Value::Null),
            "candidate_failures": preflight.and_then(|value| value.get("candidate_failures")).cloned().unwrap_or_else(|| json!([])),
        },
        "topology": {
            "class": topology_class,
            "ssh_jump_chain": topology.and_then(|value| value.get("ssh_jump_chain")).cloned().unwrap_or_else(|| json!([])),
            "direct_private_candidates": topology.and_then(|value| value.get("direct_private_candidates")).cloned().unwrap_or_else(|| json!([])),
        },
        "policy": {
            "direct_transport_policy": direct_transport_policy,
            "direct_transport_policy_reason": direct_transport_policy_reason,
            "tls_peer_auth_mode": tls_peer_auth_mode,
            "ssh_data_plane_reason": plan
                .get("ssh_data_plane_reason")
                .cloned()
                .unwrap_or(Value::Null),
            "explicit_user_override": matches!(selection_source, "cli" | "profile"),
            "selection_source": selection_source,
        },
        "workload": {
            "hint": runtime.and_then(|value| value.get("workload_hint")).cloned().unwrap_or(Value::Null),
            "pool_policy": runtime.and_then(|value| value.get("pool_policy")).cloned().unwrap_or(Value::Null),
            "transport_pool_size": runtime.and_then(|value| value.get("transport_pool_size")).cloned().unwrap_or(Value::Null),
        },
        "selected_transport": selected_transport,
        "selected_reason": selection_reason,
        "fallback_reason": fallback_reason,
        "next_action": next_action,
    });
    if let Some(object) = plan.as_object_mut() {
        object.insert("decision_chain".to_string(), decision_chain);
    }
}

fn route_topology_class(preflight: Option<&Value>, topology: Option<&Value>) -> &'static str {
    let direct_reachable = preflight
        .and_then(|value| value.get("results"))
        .and_then(Value::as_array)
        .map(|results| {
            results.iter().any(|result| {
                is_direct_probe_protocol(result.get("protocol").and_then(Value::as_str))
                    && result.get("reachable") == Some(&Value::Bool(true))
            })
        })
        .unwrap_or(false);
    if direct_reachable {
        return "direct-reachable";
    }

    let recommended_fallback = preflight
        .and_then(|value| value.get("recommended_fallback"))
        .and_then(Value::as_str);
    if recommended_fallback.is_some() {
        return "ssh-only";
    }

    let has_jump = topology
        .and_then(|value| value.get("ssh_jump_chain"))
        .and_then(Value::as_array)
        .map(|chain| !chain.is_empty())
        .unwrap_or(false);
    let has_direct_candidates = topology
        .and_then(|value| value.get("direct_private_candidates"))
        .and_then(Value::as_array)
        .map(|candidates| !candidates.is_empty())
        .unwrap_or(false);
    if has_jump && has_direct_candidates {
        return "mixed";
    }
    if has_direct_candidates {
        return "unknown-direct";
    }
    "ssh-reachable"
}

fn transport_candidates(forward: &cli::NodeForwardArgs) -> Vec<String> {
    let mut candidates = Vec::new();
    if forward.remote_quic.is_some() {
        candidates.push("quic".to_string());
    }
    if forward.remote_tls.is_some() {
        candidates.push("tls-tcp".to_string());
    }
    if forward.allow_plain_tcp {
        candidates.push("plain-tcp".to_string());
    }
    candidates.push("ssh-native".to_string());
    candidates.push("ssh-direct-tcpip".to_string());
    candidates.push("ssh-exec".to_string());
    candidates
}

fn topology_hint(args: &cli::RouteArgs, forward: &cli::NodeForwardArgs) -> Value {
    let ssh_target = ssh_client::resolve_route_target(args);
    let (ssh_host, ssh_jump_chain) = match ssh_target {
        Ok(target) => (
            Some(format!("{}:{}", target.host, target.port)),
            target
                .jumps
                .into_iter()
                .map(|jump| format!("{}@{}:{}", jump.user, jump.host, jump.port))
                .collect::<Vec<_>>(),
        ),
        Err(err) => {
            return json!({
                "ssh_target": Value::Null,
                "ssh_jump_chain": [],
                "direct_private_candidates": direct_private_candidates(forward),
                "warning": format!("failed to resolve SSH target for topology hint: {err}"),
            });
        }
    };
    let direct_private_candidates = direct_private_candidates(forward);
    let warning = if !ssh_jump_chain.is_empty() && !direct_private_candidates.is_empty() {
        Some(
            "SSH target uses ProxyJump; direct QUIC/TLS/plain peer endpoints do not automatically traverse the SSH jump path and may be unreachable. Prefer SSH fallback or a reachable peer endpoint."
                .to_string(),
        )
    } else if direct_private_candidates
        .iter()
        .any(|candidate| candidate.ends_with("127.0.0.1") || candidate.contains("127.0.0.1:"))
    {
        Some(
            "direct peer endpoint is loopback; it is reachable only from the same machine or through SSH direct-tcpip fallback"
                .to_string(),
        )
    } else {
        None
    };
    json!({
        "ssh_target": ssh_host,
        "ssh_jump_chain": ssh_jump_chain,
        "direct_private_candidates": direct_private_candidates,
        "warning": warning,
    })
}

fn direct_private_candidates(forward: &cli::NodeForwardArgs) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(addr) = forward.remote_quic {
        candidates.push(format!("quic://{addr}"));
    }
    if let Some(addr) = forward.remote_tls {
        candidates.push(format!("tls-tcp://{addr}"));
    }
    if forward.allow_plain_tcp {
        candidates.push(format!("plain-tcp://{}", forward.remote_tcp));
    }
    candidates
}
