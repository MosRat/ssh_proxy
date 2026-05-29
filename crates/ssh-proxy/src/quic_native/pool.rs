use serde_json::{Map, Value};

use crate::cli;

pub(super) const QUIC_CONNECTION_POOL_SELECTION_POLICY: &str = "least-loaded by active streams, stream open failures, latency, backpressure, resets, and control health";

pub(super) fn quic_connection_pool_policy(args: &cli::ProxyArgs) -> String {
    args.pool_policy
        .clone()
        .unwrap_or_else(|| "explicit".to_string())
}

pub(super) fn quic_connection_pool_workload_hint(args: &cli::ProxyArgs) -> Option<&'static str> {
    args.workload_hint.map(workload_hint_name).or_else(|| {
        if args.tcp_target.is_some() {
            Some("large")
        } else if args.transport_pool_size.max(1) > 1 {
            Some("concurrent")
        } else {
            None
        }
    })
}

pub(super) fn quic_connection_pool_reason(args: &cli::ProxyArgs) -> String {
    if let Some(reason) = args.transport_pool_reason.as_deref() {
        return reason.to_string();
    }
    match (
        args.tcp_target.is_some(),
        quic_connection_pool_workload_hint(args),
    ) {
        (true, Some("large")) => {
            "fixed --tcp-target route keeps one QUIC connection unless explicitly configured"
                .to_string()
        }
        (false, Some("large")) => "single-flow route keeps one QUIC connection".to_string(),
        (false, Some("concurrent")) => {
            "multi-flow proxy route uses a workload-aware QUIC connection pool".to_string()
        }
        (false, Some("mixed")) => {
            "mixed-workload proxy route uses a workload-aware QUIC connection pool".to_string()
        }
        _ => "QUIC-native route uses a single connection".to_string(),
    }
}

pub(super) fn quic_connection_pool_mode(pool_size: usize) -> &'static str {
    if pool_size > 1 {
        "workload-aware-pool"
    } else {
        "single-connection"
    }
}

pub(super) fn insert_zero_quic_lifecycle_status(status: &mut Map<String, Value>) {
    for key in [
        "quic_stream_open_samples",
        "quic_stream_open_failures",
        "quic_header_write_samples",
        "quic_header_write_failures",
        "quic_backpressure_timeouts",
        "quic_flow_graceful_closes",
        "quic_flow_resets",
        "quic_flow_first_byte_samples",
        "quic_copy_duration_samples",
        "quic_copy_failures",
    ] {
        status.insert(key.to_string(), 0.into());
    }
    for key in [
        "last_quic_stream_open_latency_ms",
        "max_quic_stream_open_latency_ms",
        "last_quic_header_write_latency_ms",
        "max_quic_header_write_latency_ms",
        "last_quic_flow_first_byte_latency_ms",
        "max_quic_flow_first_byte_latency_ms",
        "last_quic_copy_duration_ms",
        "max_quic_copy_duration_ms",
        "last_quic_copy_client_to_remote_bytes",
        "last_quic_copy_remote_to_client_bytes",
        "max_quic_copy_client_to_remote_bytes",
        "max_quic_copy_remote_to_client_bytes",
    ] {
        status.insert(key.to_string(), Value::Null);
    }
}

pub(super) fn quic_profile_next_bottleneck(
    control_degraded: bool,
    quic_backpressure_timeouts: u64,
    quic_copy_failures: u64,
    quic_copy_duration_samples: u64,
    quic_stream_open_failures: u64,
    quic_header_write_failures: u64,
) -> &'static str {
    if control_degraded {
        "control_stream"
    } else if quic_backpressure_timeouts > 0 {
        "slow_consumers"
    } else if quic_copy_failures > 0 || quic_copy_duration_samples > 0 {
        "application_copy"
    } else if quic_stream_open_failures > 0 || quic_header_write_failures > 0 {
        "window_sizing"
    } else {
        "udp_path_or_congestion"
    }
}

pub(super) fn quic_worker_score(
    active_flows: u64,
    stream_open_failures: u64,
    last_open_latency_ms: u64,
    backpressure_timeouts: u64,
    resets: u64,
    control_degraded: bool,
) -> u64 {
    active_flows
        .saturating_mul(10_000)
        .saturating_add(stream_open_failures.saturating_mul(1_000))
        .saturating_add(backpressure_timeouts.saturating_mul(2_000))
        .saturating_add(resets.saturating_mul(1_500))
        .saturating_add(last_open_latency_ms.min(5_000))
        .saturating_add(if control_degraded { 1_000_000 } else { 0 })
}

fn workload_hint_name(hint: cli::RouteWorkloadHint) -> &'static str {
    match hint {
        cli::RouteWorkloadHint::Large => "large",
        cli::RouteWorkloadHint::Concurrent => "concurrent",
        cli::RouteWorkloadHint::Mixed => "mixed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quic_worker_score_prefers_less_busy_healthy_connections() {
        let idle = quic_worker_score(0, 0, 10, 0, 0, false);
        let busy = quic_worker_score(2, 0, 10, 0, 0, false);
        let failed = quic_worker_score(0, 3, 10, 0, 0, false);
        let backed_up = quic_worker_score(0, 0, 10, 2, 0, false);
        let reset = quic_worker_score(0, 0, 10, 0, 2, false);
        let degraded = quic_worker_score(0, 0, 10, 0, 0, true);

        assert!(idle < busy);
        assert!(idle < failed);
        assert!(idle < backed_up);
        assert!(idle < reset);
        assert!(busy < degraded);
    }

    #[test]
    fn quic_profile_next_bottleneck_prefers_backpressure_then_copy_then_window() {
        assert_eq!(
            quic_profile_next_bottleneck(true, 0, 0, 0, 0, 0),
            "control_stream"
        );
        assert_eq!(
            quic_profile_next_bottleneck(false, 2, 0, 0, 0, 0),
            "slow_consumers"
        );
        assert_eq!(
            quic_profile_next_bottleneck(false, 0, 1, 1, 0, 0),
            "application_copy"
        );
        assert_eq!(
            quic_profile_next_bottleneck(false, 0, 0, 0, 1, 0),
            "window_sizing"
        );
        assert_eq!(
            quic_profile_next_bottleneck(false, 0, 0, 0, 0, 0),
            "udp_path_or_congestion"
        );
    }
}
