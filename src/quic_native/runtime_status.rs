use serde_json::{Value, json};

use crate::{peer_transport, protocol};

use super::pool::{QUIC_CONNECTION_POOL_SELECTION_POLICY, insert_zero_quic_lifecycle_status};

pub(super) fn disconnected_status_value(
    last_error: Option<String>,
    control_keepalive_interval_secs: u64,
    control_keepalive_timeout_secs: u64,
    copy_buffer_size: usize,
    first_byte_timeout_secs: u64,
    backpressure_timeout_secs: u64,
) -> Value {
    let mut status = serde_json::Map::new();
    status.insert("connected".to_string(), false.into());
    status.insert("selected_protocol".to_string(), "quic-native".into());
    status.insert("quic_mode".to_string(), "native-per-flow".into());
    status.insert("quic_connection_pool_size".to_string(), 0.into());
    status.insert(
        "quic_connection_pool_policy".to_string(),
        "disconnected".into(),
    );
    status.insert(
        "quic_connection_pool_workload_hint".to_string(),
        Value::Null,
    );
    status.insert(
        "quic_connection_pool_reason".to_string(),
        "no active QUIC-native route".into(),
    );
    status.insert(
        "quic_connection_pool_mode".to_string(),
        "disconnected".into(),
    );
    status.insert(
        "quic_connection_pool_selection_policy".to_string(),
        QUIC_CONNECTION_POOL_SELECTION_POLICY.into(),
    );
    status.insert("active_quic_connections".to_string(), 0.into());
    status.insert("quic_connections".to_string(), Value::Array(Vec::new()));
    status.insert("active_quic_flows".to_string(), 0.into());
    insert_zero_quic_lifecycle_status(&mut status);
    status.insert("active_tcp".to_string(), 0.into());
    status.insert("total_tcp".to_string(), 0.into());
    status.insert("tcp_open_attempts".to_string(), 0.into());
    status.insert("tcp_open_successes".to_string(), 0.into());
    status.insert("tcp_open_failures".to_string(), 0.into());
    status.insert("bytes_client_to_remote".to_string(), 0.into());
    status.insert("bytes_remote_to_client".to_string(), 0.into());
    status.insert("control_state".to_string(), "disconnected".into());
    status.insert("control_degraded".to_string(), true.into());
    status.insert("control_pings_sent".to_string(), 0.into());
    status.insert("control_pongs_received".to_string(), 0.into());
    status.insert("last_control_pong_latency_ms".to_string(), Value::Null);
    status.insert(
        "control_keepalive_interval_secs".to_string(),
        control_keepalive_interval_secs.into(),
    );
    status.insert(
        "control_keepalive_timeout_secs".to_string(),
        control_keepalive_timeout_secs.into(),
    );
    status.insert(
        "last_control_error".to_string(),
        serde_json::to_value(last_error.clone()).expect("control error serializable"),
    );
    status.insert("read_buffer_size".to_string(), copy_buffer_size.into());
    status.insert("quic_copy_buffer_size".to_string(), copy_buffer_size.into());
    status.insert("quic_stream_open_timeout_secs".to_string(), Value::Null);
    status.insert(
        "quic_first_byte_timeout_secs".to_string(),
        first_byte_timeout_secs.into(),
    );
    status.insert(
        "quic_backpressure_timeout_secs".to_string(),
        backpressure_timeout_secs.into(),
    );
    status.insert(
        "write_batch_limit".to_string(),
        protocol::FRAME_WRITE_BATCH_LIMIT.into(),
    );
    status.insert(
        "frame_channel_capacity".to_string(),
        protocol::FRAME_CHANNEL_CAPACITY.into(),
    );
    status.insert(
        "quic_receive_window".to_string(),
        peer_transport::QUIC_RECEIVE_WINDOW.into(),
    );
    status.insert(
        "quic_stream_receive_window".to_string(),
        peer_transport::QUIC_STREAM_RECEIVE_WINDOW.into(),
    );
    status.insert(
        "quic_max_bidi_streams".to_string(),
        peer_transport::QUIC_MAX_BIDI_STREAMS.into(),
    );
    status.insert(
        "quic_keep_alive_interval_secs".to_string(),
        peer_transport::QUIC_KEEP_ALIVE_INTERVAL_SECS.into(),
    );
    status.insert(
        "quic_idle_timeout_secs".to_string(),
        peer_transport::QUIC_IDLE_TIMEOUT_SECS.into(),
    );
    status.insert(
        "quic_runtime".to_string(),
        serde_json::to_value(peer_transport::quic_runtime_diagnostics(
            peer_transport::QuicTransportOptions::default(),
        ))
        .expect("quic runtime diagnostics serializable"),
    );
    status.insert(
        "quic_udp_runtime".to_string(),
        peer_transport::QUIC_UDP_RUNTIME.into(),
    );
    status.insert("quic_udp_gso".to_string(), Value::Null);
    status.insert(
        "quic_udp_gso_source".to_string(),
        peer_transport::QUIC_UDP_GSO_SOURCE.into(),
    );
    status.insert(
        "quic_packetization".to_string(),
        peer_transport::QUIC_PACKETIZATION.into(),
    );
    status.insert(
        "quic_profile".to_string(),
        json!({
            "enabled": true,
            "profile_scope": "quic-native-route-status",
            "connected": false,
            "mode": "native-per-flow",
            "pool": {
                "size": 0,
                "active_connections": 0,
                "policy": "disconnected",
                "workload_hint": Value::Null,
                "reason": "no active QUIC-native route",
                "mode": "disconnected",
                "selection_policy": QUIC_CONNECTION_POOL_SELECTION_POLICY,
            },
            "transport": {
                "max_bidi_streams": peer_transport::QUIC_MAX_BIDI_STREAMS,
                "stream_receive_window": peer_transport::QUIC_STREAM_RECEIVE_WINDOW,
                "receive_window": peer_transport::QUIC_RECEIVE_WINDOW,
                "keep_alive_interval_secs": peer_transport::QUIC_KEEP_ALIVE_INTERVAL_SECS,
                "idle_timeout_secs": peer_transport::QUIC_IDLE_TIMEOUT_SECS,
            },
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
            "connections": [],
            "flow": {
                "stream_open_samples": 0,
                "stream_open_failures": 0,
                "header_write_samples": 0,
                "header_write_failures": 0,
                "first_byte_samples": 0,
                "copy_duration_samples": 0,
                "copy_failures": 0,
                "backpressure_timeouts": 0,
                "graceful_closes": 0,
                "resets": 0,
            },
            "copy": {
                "buffer_size": copy_buffer_size,
                "duration_samples": 0,
                "failures": 0,
                "backpressure_timeouts": 0,
            },
            "signals": {
                "next_bottleneck": "disconnected",
                "window_sizing": { "suspected": false, "evidence": "no active QUIC route" },
                "udp_path": { "suspected": false, "evidence": "no active QUIC route" },
                "application_copy": { "suspected": false, "evidence": "no active QUIC route" },
                "slow_consumers": { "suspected": false, "evidence": "no active QUIC route" },
                "congestion": { "suspected": false, "evidence": "no active QUIC route" },
            },
        }),
    );
    status.insert("last_quic_flow_close_reason".to_string(), Value::Null);
    status.insert(
        "last_error".to_string(),
        serde_json::to_value(last_error.clone()).expect("last error serializable"),
    );
    status.insert(
        "link".to_string(),
        json!({
            "health": {
                "selected_protocol": "quic-native",
                "active_connections": 0,
                "active_streams": 0,
                "active_channels": 0,
                "pool_size": 0,
                "pool_policy": "disconnected",
                "pool_workload_hint": Value::Null,
                "pool_reason": "no active QUIC-native route",
                "pool_mode": "disconnected",
                "pool_selection_policy": QUIC_CONNECTION_POOL_SELECTION_POLICY,
                "open_attempts": 0,
                "open_successes": 0,
                "open_failures": 0,
                "open_latency_ms": Value::Null,
                "bytes_client_to_remote": 0,
                "bytes_remote_to_client": 0,
                "first_byte_samples": 0,
                "first_byte_latency_ms": Value::Null,
                "max_first_byte_latency_ms": Value::Null,
                "last_close_reason": Value::Null,
                "degraded_reason": last_error,
                "control_health": "disconnected",
                "connected": false,
            }
        }),
    );
    Value::Object(status)
}
