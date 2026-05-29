use serde_json::{Value, json};

use crate::{
    peer_transport,
    quic_native::pool::{
        QUIC_CONNECTION_POOL_SELECTION_POLICY, quic_connection_pool_mode,
        quic_connection_pool_policy, quic_connection_pool_reason,
        quic_connection_pool_workload_hint, quic_profile_next_bottleneck,
    },
};

use super::{super::State, snapshot::QuicStatusSnapshot};

pub(super) fn quic_profile_value(state: &State, snapshot: &QuicStatusSnapshot) -> Value {
    json!({
        "enabled": true,
        "profile_scope": "quic-native-route-status",
        "connected": snapshot.connected,
        "mode": "native-per-flow",
        "route_id": state.route_id.clone(),
        "active_connections": snapshot.active_quic_connections,
        "active_flows": snapshot.active_quic_flows,
        "pool": {
            "size": state.args.transport_pool_size.max(1),
            "active_connections": snapshot.active_quic_connections,
            "policy": quic_connection_pool_policy(&state.args),
            "workload_hint": quic_connection_pool_workload_hint(&state.args),
            "reason": quic_connection_pool_reason(&state.args),
            "mode": quic_connection_pool_mode(state.args.transport_pool_size.max(1)),
            "selection_policy": QUIC_CONNECTION_POOL_SELECTION_POLICY,
        },
        "transport": snapshot.quic_runtime.transport_options,
        "runtime": snapshot.quic_runtime.clone(),
        "udp": {
            "runtime": peer_transport::QUIC_UDP_RUNTIME,
            "gso": Value::Null,
            "gso_source": peer_transport::QUIC_UDP_GSO_SOURCE,
            "packetization": peer_transport::QUIC_PACKETIZATION,
            "max_datagram_size": Value::Null,
            "max_datagram_size_source": "unavailable: Quinn endpoint API is not exposed through ssh_proxy status",
            "packet_loss": Value::Null,
            "packet_loss_source": "unavailable: Quinn connection loss counters are not exposed through ssh_proxy status",
            "congestion_signal": Value::Null,
            "congestion_signal_source": "unavailable: Quinn congestion counters are not exposed through ssh_proxy status",
        },
        "connections": snapshot.quic_connections.clone(),
        "flow": {
            "stream_open_samples": snapshot.quic_stream_open_samples,
            "stream_open_failures": snapshot.quic_stream_open_failures,
            "last_stream_open_latency_ms": snapshot.last_quic_stream_open_latency_ms,
            "max_stream_open_latency_ms": snapshot.max_quic_stream_open_latency_ms,
            "header_write_samples": snapshot.quic_header_write_samples,
            "header_write_failures": snapshot.quic_header_write_failures,
            "last_header_write_latency_ms": snapshot.last_quic_header_write_latency_ms,
            "max_header_write_latency_ms": snapshot.max_quic_header_write_latency_ms,
            "first_byte_samples": snapshot.first_byte_samples,
            "last_first_byte_latency_ms": snapshot.last_quic_flow_first_byte_latency_ms,
            "max_first_byte_latency_ms": snapshot.max_quic_flow_first_byte_latency_ms,
            "copy_duration_samples": snapshot.quic_copy_duration_samples,
            "last_copy_duration_ms": snapshot.last_quic_copy_duration_ms,
            "max_copy_duration_ms": snapshot.max_quic_copy_duration_ms,
            "copy_failures": snapshot.quic_copy_failures,
            "last_copy_client_to_remote_bytes": snapshot.last_quic_copy_client_to_remote_bytes,
            "last_copy_remote_to_client_bytes": snapshot.last_quic_copy_remote_to_client_bytes,
            "max_copy_client_to_remote_bytes": snapshot.max_quic_copy_client_to_remote_bytes,
            "max_copy_remote_to_client_bytes": snapshot.max_quic_copy_remote_to_client_bytes,
            "backpressure_timeouts": snapshot.quic_backpressure_timeouts,
            "graceful_closes": snapshot.quic_flow_graceful_closes,
            "resets": snapshot.quic_flow_resets,
        },
        "control": {
            "state": snapshot.control_state,
            "degraded": snapshot.control_degraded,
            "pings_sent": snapshot.control_pings_sent,
            "pongs_received": snapshot.control_pongs_received,
            "last_pong_latency_ms": snapshot.last_control_pong_latency_ms,
            "last_error": snapshot.last_control_error.clone(),
        },
        "signals": {
            "next_bottleneck": quic_profile_next_bottleneck(
                snapshot.control_degraded,
                snapshot.quic_backpressure_timeouts,
                snapshot.quic_copy_failures,
                snapshot.quic_copy_duration_samples,
                snapshot.quic_stream_open_failures,
                snapshot.quic_header_write_failures,
            ),
            "window_sizing": {
                "suspected": snapshot.quic_stream_open_failures > 0 || snapshot.quic_header_write_failures > 0,
                "evidence": if snapshot.quic_stream_open_failures > 0 || snapshot.quic_header_write_failures > 0 {
                    "stream or header opens are failing"
                } else if snapshot.quic_stream_open_samples == 0 {
                    "no successful QUIC stream opens recorded"
                } else {
                    "stream opens are currently healthy"
                },
            },
            "udp_path": {
                "suspected": snapshot.quic_runtime.udp_gso.is_none(),
                "evidence": snapshot.quic_runtime.udp_gso_source,
            },
            "application_copy": {
                "suspected": snapshot.quic_copy_failures > 0 || snapshot.quic_copy_duration_samples > 0,
                "evidence": if snapshot.quic_copy_failures > 0 {
                    "copy failures were recorded"
                } else if snapshot.quic_copy_duration_samples > 0 {
                    "copy duration samples are present"
                } else {
                    "no copy samples recorded"
                },
            },
            "slow_consumers": {
                "suspected": snapshot.quic_backpressure_timeouts > 0,
                "evidence": if snapshot.quic_backpressure_timeouts > 0 {
                    "backpressure timeouts were recorded"
                } else {
                    "no backpressure timeouts recorded"
                },
            },
            "congestion": {
                "suspected": false,
                "evidence": "quinn congestion counters are not exposed through the current status surface",
            },
        },
    })
}

pub(super) fn link_value(state: &State, snapshot: &QuicStatusSnapshot) -> Value {
    json!({
        "health": {
            "selected_protocol": "quic-native",
            "active_connections": snapshot.active_quic_connections,
            "active_streams": snapshot.active_quic_flows,
            "active_channels": snapshot.active_quic_flows,
            "pool_size": state.args.transport_pool_size.max(1),
            "pool_policy": quic_connection_pool_policy(&state.args),
            "pool_workload_hint": quic_connection_pool_workload_hint(&state.args),
            "pool_reason": quic_connection_pool_reason(&state.args),
            "pool_mode": quic_connection_pool_mode(state.args.transport_pool_size.max(1)),
            "pool_selection_policy": QUIC_CONNECTION_POOL_SELECTION_POLICY,
            "open_attempts": snapshot.quic_stream_open_samples + snapshot.quic_stream_open_failures,
            "open_successes": snapshot.quic_stream_open_samples,
            "open_failures": snapshot.quic_stream_open_failures,
            "open_latency_ms": snapshot.last_quic_stream_open_latency_ms,
            "bytes_client_to_remote": snapshot.bytes_client_to_remote,
            "bytes_remote_to_client": snapshot.bytes_remote_to_client,
            "first_byte_samples": snapshot.first_byte_samples,
            "first_byte_latency_ms": snapshot.last_quic_flow_first_byte_latency_ms,
            "max_first_byte_latency_ms": snapshot.max_quic_flow_first_byte_latency_ms,
            "last_close_reason": snapshot.last_quic_flow_close_reason.clone(),
            "degraded_reason": snapshot
                .last_control_error
                .clone()
                .or(snapshot.last_error.clone()),
            "control_health": snapshot.control_state,
            "connected": snapshot.connected,
        }
    })
}
