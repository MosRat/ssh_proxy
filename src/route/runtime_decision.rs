use serde_json::{Value, json};

use crate::cli;

use super::transport::{
    direct_transport_policy, direct_transport_policy_reason, remote_transport_name,
    ssh_data_plane_reason, ssh_mode_name, ssh_mode_reason, tls_peer_auth_mode,
};

pub(crate) struct RouteRuntimeDecision {
    value: Value,
}

impl RouteRuntimeDecision {
    pub(crate) fn from_forward_task(proxy: &cli::ProxyArgs) -> Self {
        Self {
            value: forward_route_runtime_metadata(proxy),
        }
    }

    pub(crate) fn into_value(self) -> Value {
        self.value
    }
}

pub(crate) fn forward_route_runtime_metadata(proxy: &cli::ProxyArgs) -> Value {
    json!({
        "selected_transport": remote_transport_name(proxy.remote_transport),
        "transport_selection_source": proxy.transport_selection_source.as_deref().unwrap_or("unknown"),
        "transport_selection_reason": proxy.transport_selection_reason.as_deref().unwrap_or("daemon received an already-materialized forward route task"),
        "direct_transport_policy": direct_transport_policy(proxy.remote_transport),
        "direct_transport_policy_reason": direct_transport_policy_reason(proxy.remote_transport),
        "tls_peer_auth_mode": tls_peer_auth_mode(
            proxy.remote_transport,
            proxy.remote_client_cert.as_ref(),
            proxy.remote_client_key.as_ref(),
        ),
        "preflight": preflight_metadata(proxy),
        "decision_chain": runtime_decision_chain(proxy),
        "ssh_mode": ssh_mode_name(proxy.remote_transport),
        "ssh_mode_reason": ssh_mode_reason(proxy.remote_transport),
        "ssh_data_plane_reason": ssh_data_plane_reason(
            proxy.remote_transport,
            proxy.transport_selection_source.as_deref(),
        ),
        "ssh_session_pool_size": if matches!(proxy.remote_transport, cli::RemoteTransport::SshNative) {
            Value::from(proxy.ssh_session_pool_size.unwrap_or(1))
        } else {
            Value::Null
        },
        "ssh_session_pool_source": if matches!(proxy.remote_transport, cli::RemoteTransport::SshNative) {
            Value::from(proxy.ssh_session_pool_source.as_deref().unwrap_or("unknown"))
        } else {
            Value::Null
        },
        "ssh_session_pool_reason": if matches!(proxy.remote_transport, cli::RemoteTransport::SshNative) {
            Value::from(proxy.ssh_session_pool_reason.as_deref().unwrap_or("unknown"))
        } else {
            Value::Null
        },
        "ssh_session_pool_warning": if matches!(proxy.remote_transport, cli::RemoteTransport::SshNative) {
            json!(proxy.ssh_session_pool_warning.as_deref())
        } else {
            Value::Null
        },
        "transport_pool_size": proxy.transport_pool_size,
        "transport_pool_source": proxy.transport_pool_source.as_deref().unwrap_or("route-task"),
        "transport_pool_reason": proxy.transport_pool_reason.as_deref().unwrap_or("daemon received an already-materialized forward route task"),
        "pool_policy": proxy.pool_policy.as_deref().unwrap_or("explicit"),
        "workload_hint": proxy.workload_hint.map(workload_hint_name),
        "connect_timeout_secs": proxy.connect_timeout_secs,
        "reconnect_delay_secs": proxy.reconnect_delay_secs,
        "reconnect_max_delay_secs": proxy.reconnect_max_delay_secs,
        "no_reconnect": proxy.no_reconnect,
    })
}

fn preflight_metadata(proxy: &cli::ProxyArgs) -> Value {
    let has_any = proxy.preflight_recommended_fallback.is_some()
        || proxy.preflight_selected_reason.is_some()
        || proxy.preflight_repair_hint.is_some()
        || !proxy.preflight_candidate_failures.is_empty();
    if !has_any {
        return Value::Null;
    }
    json!({
        "recommended_fallback": proxy.preflight_recommended_fallback.as_deref(),
        "selected_reason": proxy.preflight_selected_reason.as_deref().unwrap_or("unknown"),
        "repair_hint": proxy.preflight_repair_hint.as_deref().unwrap_or("unknown"),
        "candidate_failures": proxy.preflight_candidate_failures.clone(),
    })
}

fn runtime_decision_chain(proxy: &cli::ProxyArgs) -> Value {
    let preflight = preflight_metadata(proxy);
    let topology_class = if proxy.preflight_recommended_fallback.is_some() {
        "ssh-only"
    } else {
        "runtime-materialized"
    };
    json!({
        "preflight": preflight,
        "topology": {
            "class": topology_class,
        },
        "policy": {
            "direct_transport_policy": direct_transport_policy(proxy.remote_transport),
            "direct_transport_policy_reason": direct_transport_policy_reason(proxy.remote_transport),
            "tls_peer_auth_mode": tls_peer_auth_mode(
                proxy.remote_transport,
                proxy.remote_client_cert.as_ref(),
                proxy.remote_client_key.as_ref(),
            ),
            "ssh_data_plane_reason": ssh_data_plane_reason(
                proxy.remote_transport,
                proxy.transport_selection_source.as_deref(),
            ),
            "explicit_user_override": matches!(
                proxy.transport_selection_source.as_deref(),
                Some("cli" | "profile")
            ),
            "selection_source": proxy.transport_selection_source.as_deref().unwrap_or("unknown"),
        },
        "workload": {
            "hint": proxy.workload_hint.map(workload_hint_name),
            "pool_policy": proxy.pool_policy.as_deref().unwrap_or("explicit"),
            "transport_pool_size": proxy.transport_pool_size,
        },
        "selected_transport": remote_transport_name(proxy.remote_transport),
        "selected_reason": proxy.transport_selection_reason.as_deref().unwrap_or("daemon received an already-materialized forward route task"),
        "fallback_reason": if proxy.preflight_recommended_fallback.is_some() {
            proxy.transport_selection_reason.as_deref()
        } else {
            None
        },
        "next_action": if proxy.preflight_recommended_fallback.is_some() {
            "using materialized preflight selection"
        } else {
            "none"
        },
    })
}

fn workload_hint_name(hint: cli::RouteWorkloadHint) -> &'static str {
    match hint {
        cli::RouteWorkloadHint::Large => "large",
        cli::RouteWorkloadHint::Concurrent => "concurrent",
        cli::RouteWorkloadHint::Mixed => "mixed",
    }
}
