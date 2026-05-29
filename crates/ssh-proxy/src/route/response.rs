use std::net::SocketAddr;

use serde_json::{Value, json};
use ssh_proxy_core::model::TransportMode;
use ssh_proxy_route::{
    RoutePlanReport, RouteRuntimePlanReport, SshSessionPoolReport, direct_transport_policy,
    direct_transport_policy_reason, remote_transport_name, ssh_data_plane_reason, ssh_mode_name,
    ssh_mode_reason, tls_peer_auth_mode,
};

use crate::{cli, ssh_client};

use super::policy::pool_policy_name;

pub(crate) fn local_uses_remote_plan(
    args: &cli::RouteArgs,
    id: &str,
    forward: &cli::NodeForwardArgs,
) -> Value {
    let transport = TransportMode::from(forward.remote_transport);
    RoutePlanReport {
        route_id: id.to_string(),
        direction: "local-uses-remote".to_string(),
        owner: "local".to_string(),
        mode: "local-forward".to_string(),
        listener: json!({
            "owner": "local",
            "listen": forward.listen.to_string(),
            "tcp_target": forward.tcp_target.as_ref().map(ToString::to_string),
        }),
        egress: json!({
            "peer": args.target,
            "side": "remote",
            "upstream_proxy": forward.egress_proxy.clone(),
        }),
        transport_candidates: transport_candidates(forward),
        selected_transport: remote_transport_name(transport).to_string(),
        transport_selection_source: Some(
            forward
                .transport_selection_source
                .as_deref()
                .unwrap_or("unknown")
                .to_string(),
        ),
        transport_selection_reason: Some(
            forward
                .transport_selection_reason
                .as_deref()
                .unwrap_or("unknown")
                .to_string(),
        ),
        direct_transport_policy: direct_transport_policy(transport),
        direct_transport_policy_reason: direct_transport_policy_reason(transport),
        tls_peer_auth_mode: tls_peer_auth_mode(
            transport,
            forward.remote_client_cert.is_some(),
            forward.remote_client_key.is_some(),
        ),
        ssh_mode: ssh_mode_name(transport),
        ssh_mode_reason: ssh_mode_reason(transport),
        ssh_data_plane_reason: ssh_data_plane_reason(
            transport,
            forward.transport_selection_source.as_deref(),
        ),
        include_ssh_session_pool_fields: true,
        ssh_session_pool: ssh_session_pool_report(forward),
        topology: Some(topology_hint(args, forward)),
        preflight: None,
        runtime: route_runtime_plan_report(
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
        fallback_reason: None,
        next_action: "none".to_string(),
        persist: !args.volatile,
    }
    .to_json()
}

pub(crate) fn remote_uses_local_reverse_link_plan(
    args: &cli::RouteArgs,
    id: &str,
    reverse: &cli::NodeReverseArgs,
    fallback_reason: Option<&str>,
) -> Value {
    RoutePlanReport {
        route_id: id.to_string(),
        direction: "remote-uses-local".to_string(),
        owner: "local".to_string(),
        mode: "reverse-link".to_string(),
        listener: json!({
            "owner": "remote",
            "listen": reverse.remote_listen.to_string(),
            "tcp_target": reverse.tcp_target.as_ref().map(ToString::to_string),
        }),
        egress: json!({
            "peer": "local",
            "side": "local",
            "upstream_proxy": reverse.egress_proxy.clone(),
        }),
        transport_candidates: vec!["ssh-reverse-link".to_string()],
        selected_transport: "ssh-reverse-link".to_string(),
        transport_selection_source: None,
        transport_selection_reason: None,
        direct_transport_policy: Value::Null,
        direct_transport_policy_reason: Value::Null,
        tls_peer_auth_mode: Value::Null,
        ssh_mode: Value::Null,
        ssh_mode_reason: Value::Null,
        ssh_data_plane_reason: Value::Null,
        include_ssh_session_pool_fields: false,
        ssh_session_pool: None,
        topology: None,
        preflight: None,
        runtime: route_runtime_plan_report(
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
        fallback_reason: fallback_reason.map(ToOwned::to_owned),
        next_action: if fallback_reason.is_some() {
            "set --local-peer <reachable-ip:port> for direct mode"
        } else {
            "none"
        }
        .to_string(),
        persist: !args.volatile,
    }
    .to_json()
}

pub(crate) fn remote_uses_local_direct_plan(
    args: &cli::RouteArgs,
    id: &str,
    forward: &cli::NodeForwardArgs,
    local_peer: SocketAddr,
) -> Value {
    let transport = TransportMode::from(forward.remote_transport);
    RoutePlanReport {
        route_id: id.to_string(),
        direction: "remote-uses-local".to_string(),
        owner: "remote".to_string(),
        mode: "direct".to_string(),
        listener: json!({
            "owner": "remote",
            "listen": forward.listen.to_string(),
            "tcp_target": forward.tcp_target.as_ref().map(ToString::to_string),
        }),
        egress: json!({
            "peer": "local",
            "side": "local",
            "reachable_peer": local_peer.to_string(),
            "upstream_proxy": forward.egress_proxy.clone(),
        }),
        transport_candidates: transport_candidates(forward),
        selected_transport: remote_transport_name(transport).to_string(),
        transport_selection_source: Some(
            forward
                .transport_selection_source
                .as_deref()
                .unwrap_or("unknown")
                .to_string(),
        ),
        transport_selection_reason: Some(
            forward
                .transport_selection_reason
                .as_deref()
                .unwrap_or("unknown")
                .to_string(),
        ),
        direct_transport_policy: direct_transport_policy(transport),
        direct_transport_policy_reason: direct_transport_policy_reason(transport),
        tls_peer_auth_mode: tls_peer_auth_mode(
            transport,
            forward.remote_client_cert.is_some(),
            forward.remote_client_key.is_some(),
        ),
        ssh_mode: ssh_mode_name(transport),
        ssh_mode_reason: ssh_mode_reason(transport),
        ssh_data_plane_reason: ssh_data_plane_reason(
            transport,
            forward.transport_selection_source.as_deref(),
        ),
        include_ssh_session_pool_fields: true,
        ssh_session_pool: ssh_session_pool_report(forward),
        topology: Some(topology_hint(args, forward)),
        preflight: None,
        runtime: route_runtime_plan_report(
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
        fallback_reason: None,
        next_action: "none".to_string(),
        persist: !args.volatile,
    }
    .to_json()
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

fn route_runtime_plan_report(
    reconnect_delay_secs: u64,
    reconnect_max_delay_secs: u64,
    connect_timeout_secs: u64,
    transport_pool_size: usize,
    transport_pool_source: Option<&str>,
    transport_pool_reason: Option<&str>,
    pool_policy: Option<&str>,
    workload_hint: Option<&str>,
    no_reconnect: bool,
) -> RouteRuntimePlanReport {
    RouteRuntimePlanReport {
        reconnect_delay_secs,
        reconnect_max_delay_secs,
        connect_timeout_secs,
        transport_pool_size,
        transport_pool_source: transport_pool_source.unwrap_or("implicit").to_string(),
        transport_pool_reason: transport_pool_reason
            .unwrap_or("implicit single-worker default")
            .to_string(),
        pool_policy: pool_policy.unwrap_or("large").to_string(),
        workload_hint: workload_hint.unwrap_or("large").to_string(),
        no_reconnect,
    }
}

pub(super) fn refresh_decision_chain(plan: &mut Value) {
    ssh_proxy_route::refresh_route_decision_chain(plan);
}

fn ssh_session_pool_report(forward: &cli::NodeForwardArgs) -> Option<SshSessionPoolReport> {
    matches!(forward.remote_transport, cli::RemoteTransport::SshNative).then(|| {
        SshSessionPoolReport {
            size: forward.ssh_session_pool_size.unwrap_or(1),
            source: forward
                .ssh_session_pool_source
                .as_deref()
                .unwrap_or("unknown")
                .to_string(),
            reason: forward
                .ssh_session_pool_reason
                .as_deref()
                .unwrap_or("unknown")
                .to_string(),
            warning: forward.ssh_session_pool_warning.clone(),
        }
    })
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
