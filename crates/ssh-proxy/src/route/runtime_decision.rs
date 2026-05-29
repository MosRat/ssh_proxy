use serde_json::Value;
use ssh_proxy_route::{
    RoutePreflightReport, RouteRuntimeContext, SshSessionPoolReport, TransportPoolReport,
};

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
    forward_route_runtime_context(proxy).to_metadata_value()
}

fn forward_route_runtime_context(proxy: &cli::ProxyArgs) -> RouteRuntimeContext {
    RouteRuntimeContext {
        selected_transport: remote_transport_name(proxy.remote_transport).to_string(),
        transport_selection_source: proxy
            .transport_selection_source
            .as_deref()
            .unwrap_or("unknown")
            .to_string(),
        transport_selection_reason: proxy
            .transport_selection_reason
            .as_deref()
            .unwrap_or("daemon received an already-materialized forward route task")
            .to_string(),
        direct_transport_policy: direct_transport_policy(proxy.remote_transport),
        direct_transport_policy_reason: direct_transport_policy_reason(proxy.remote_transport),
        tls_peer_auth_mode: tls_peer_auth_mode(
            proxy.remote_transport,
            proxy.remote_client_cert.as_ref(),
            proxy.remote_client_key.as_ref(),
        ),
        ssh_mode: ssh_mode_name(proxy.remote_transport),
        ssh_mode_reason: ssh_mode_reason(proxy.remote_transport),
        ssh_data_plane_reason: ssh_data_plane_reason(
            proxy.remote_transport,
            proxy.transport_selection_source.as_deref(),
        ),
        requires_external_ssh: matches!(proxy.remote_transport, cli::RemoteTransport::Exec),
        selected_endpoint: selected_endpoint(proxy),
        preflight: preflight_report(proxy),
        ssh_session_pool: ssh_session_pool_report(proxy),
        transport_pool: TransportPoolReport {
            size: proxy.transport_pool_size,
            source: proxy
                .transport_pool_source
                .as_deref()
                .unwrap_or("route-task")
                .to_string(),
            reason: proxy
                .transport_pool_reason
                .as_deref()
                .unwrap_or("daemon received an already-materialized forward route task")
                .to_string(),
            pool_policy: proxy
                .pool_policy
                .as_deref()
                .unwrap_or("explicit")
                .to_string(),
        },
        workload_hint: proxy
            .workload_hint
            .map(workload_hint_name)
            .map(str::to_string),
        connect_timeout_secs: proxy.connect_timeout_secs,
        reconnect_delay_secs: proxy.reconnect_delay_secs,
        reconnect_max_delay_secs: proxy.reconnect_max_delay_secs,
        no_reconnect: proxy.no_reconnect,
    }
}

fn selected_endpoint(proxy: &cli::ProxyArgs) -> Option<String> {
    match proxy.remote_transport {
        cli::RemoteTransport::Quic | cli::RemoteTransport::QuicNative => {
            proxy.remote_quic.map(|addr| format!("quic://{addr}"))
        }
        cli::RemoteTransport::TlsTcp => proxy.remote_tls.map(|addr| format!("tls-tcp://{addr}")),
        cli::RemoteTransport::PlainTcp => Some(format!("plain-tcp://{}", proxy.remote_tcp)),
        cli::RemoteTransport::Tcp => Some(format!("ssh-direct-tcpip://{}", proxy.remote_tcp)),
        cli::RemoteTransport::Auto
        | cli::RemoteTransport::SshNative
        | cli::RemoteTransport::Exec => None,
    }
}

fn preflight_report(proxy: &cli::ProxyArgs) -> Option<RoutePreflightReport> {
    let has_any = proxy.preflight_recommended_fallback.is_some()
        || proxy.preflight_selected_reason.is_some()
        || proxy.preflight_repair_hint.is_some()
        || !proxy.preflight_candidate_failures.is_empty();
    if !has_any {
        return None;
    }
    Some(RoutePreflightReport {
        recommended_fallback: proxy.preflight_recommended_fallback.clone(),
        selected_reason: proxy.preflight_selected_reason.clone(),
        repair_hint: proxy.preflight_repair_hint.clone(),
        candidate_failures: proxy.preflight_candidate_failures.clone(),
    })
}

fn ssh_session_pool_report(proxy: &cli::ProxyArgs) -> Option<SshSessionPoolReport> {
    matches!(proxy.remote_transport, cli::RemoteTransport::SshNative).then(|| {
        SshSessionPoolReport {
            size: proxy.ssh_session_pool_size.unwrap_or(1),
            source: proxy
                .ssh_session_pool_source
                .as_deref()
                .unwrap_or("unknown")
                .to_string(),
            reason: proxy
                .ssh_session_pool_reason
                .as_deref()
                .unwrap_or("unknown")
                .to_string(),
            warning: proxy.ssh_session_pool_warning.clone(),
        }
    })
}

fn workload_hint_name(hint: cli::RouteWorkloadHint) -> &'static str {
    match hint {
        cli::RouteWorkloadHint::Large => "large",
        cli::RouteWorkloadHint::Concurrent => "concurrent",
        cli::RouteWorkloadHint::Mixed => "mixed",
    }
}
