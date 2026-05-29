use std::sync::atomic::Ordering;

use anyhow::Result;

use crate::{
    peer_transport,
    quic_native::{
        metrics::last_sampled_u64,
        pool::{
            QUIC_CONNECTION_POOL_SELECTION_POLICY, quic_connection_pool_mode,
            quic_connection_pool_policy, quic_connection_pool_reason,
            quic_connection_pool_workload_hint, quic_profile_next_bottleneck,
        },
        runtime_config::quic_options_from_proxy_args,
    },
};

use super::{
    CONTROL_KEEPALIVE_INTERVAL, CONTROL_KEEPALIVE_TIMEOUT, QUIC_NATIVE_BACKPRESSURE_TIMEOUT,
    QUIC_NATIVE_COPY_BUFFER_SIZE, QUIC_NATIVE_FIRST_BYTE_TIMEOUT, State,
    metrics_snapshot::connection_status_values,
};

pub(super) async fn status_json(state: &State) -> Result<String> {
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&status_value(state).await)?
    ))
}

pub(super) async fn status_value(state: &State) -> serde_json::Value {
    let quic_connections = connection_status_values(state).await;
    let quic_options = quic_options_from_proxy_args(&state.args).unwrap_or_default();
    let quic_runtime = peer_transport::quic_runtime_diagnostics(quic_options);
    let active_quic_connections = state.workers.len();
    let active_quic_flows = state.active_quic_flows.load(Ordering::Relaxed);
    let quic_stream_open_samples = state.quic_stream_open_samples.load(Ordering::Relaxed);
    let quic_stream_open_failures = state.quic_stream_open_failures.load(Ordering::Relaxed);
    let quic_header_write_samples = state.quic_header_write_samples.load(Ordering::Relaxed);
    let quic_header_write_failures = state.quic_header_write_failures.load(Ordering::Relaxed);
    let quic_backpressure_timeouts = state.quic_backpressure_timeouts.load(Ordering::Relaxed);
    let quic_flow_graceful_closes = state.quic_flow_graceful_closes.load(Ordering::Relaxed);
    let quic_flow_resets = state.quic_flow_resets.load(Ordering::Relaxed);
    let first_byte_samples = state.quic_flow_first_byte_samples.load(Ordering::Relaxed);
    let last_quic_flow_first_byte_latency_ms = last_sampled_u64(
        first_byte_samples,
        state
            .last_quic_flow_first_byte_latency_ms
            .load(Ordering::Relaxed),
    );
    let max_quic_flow_first_byte_latency_ms = last_sampled_u64(
        first_byte_samples,
        state
            .max_quic_flow_first_byte_latency_ms
            .load(Ordering::Relaxed),
    );
    let quic_copy_duration_samples = state.quic_copy_duration_samples.load(Ordering::Relaxed);
    let last_quic_copy_duration_ms = last_sampled_u64(
        quic_copy_duration_samples,
        state.last_quic_copy_duration_ms.load(Ordering::Relaxed),
    );
    let max_quic_copy_duration_ms = last_sampled_u64(
        quic_copy_duration_samples,
        state.max_quic_copy_duration_ms.load(Ordering::Relaxed),
    );
    let quic_copy_failures = state.quic_copy_failures.load(Ordering::Relaxed);
    let last_quic_copy_client_to_remote_bytes = last_sampled_u64(
        quic_copy_duration_samples,
        state
            .last_quic_copy_client_to_remote_bytes
            .load(Ordering::Relaxed),
    );
    let last_quic_copy_remote_to_client_bytes = last_sampled_u64(
        quic_copy_duration_samples,
        state
            .last_quic_copy_remote_to_client_bytes
            .load(Ordering::Relaxed),
    );
    let max_quic_copy_client_to_remote_bytes = last_sampled_u64(
        quic_copy_duration_samples,
        state
            .max_quic_copy_client_to_remote_bytes
            .load(Ordering::Relaxed),
    );
    let max_quic_copy_remote_to_client_bytes = last_sampled_u64(
        quic_copy_duration_samples,
        state
            .max_quic_copy_remote_to_client_bytes
            .load(Ordering::Relaxed),
    );
    let bytes_client_to_remote = state.bytes_client_to_remote.load(Ordering::Relaxed);
    let bytes_remote_to_client = state.bytes_remote_to_client.load(Ordering::Relaxed);
    let control_degraded = state.control_degraded.load(Ordering::Relaxed);
    let control_state = if control_degraded {
        "degraded"
    } else {
        "healthy"
    };
    let control_pings_sent = state.control_pings_sent.load(Ordering::Relaxed);
    let control_pongs_received = state.control_pongs_received.load(Ordering::Relaxed);
    let last_control_pong_latency_ms = last_sampled_u64(
        control_pongs_received,
        state.last_control_pong_latency_ms.load(Ordering::Relaxed),
    );
    let last_control_error = state.last_control_error.lock().await.clone();
    let last_quic_flow_close_reason = state.last_quic_flow_close_reason.lock().await.clone();
    let last_error = state.last_error.lock().await.clone();
    let mut status = serde_json::Map::new();
    status.insert(
        "connected".to_string(),
        (!state.shutdown.load(Ordering::Relaxed)).into(),
    );
    status.insert("selected_protocol".to_string(), "quic-native".into());
    status.insert("quic_mode".to_string(), "native-per-flow".into());
    status.insert("route_id".to_string(), state.route_id.clone().into());
    status.insert(
        "remote_quic".to_string(),
        serde_json::to_value(state.args.remote_quic).expect("remote_quic serializable"),
    );
    status.insert(
        "remote_name".to_string(),
        state.args.remote_name.clone().into(),
    );
    status.insert(
        "egress_proxy".to_string(),
        serde_json::to_value(state.args.egress_proxy.clone()).expect("egress proxy serializable"),
    );
    status.insert(
        "uptime_secs".to_string(),
        state.started.elapsed().as_secs().into(),
    );
    status.insert(
        "quic_connection_pool_size".to_string(),
        state.args.transport_pool_size.max(1).into(),
    );
    status.insert(
        "quic_connection_pool_policy".to_string(),
        quic_connection_pool_policy(&state.args).into(),
    );
    status.insert(
        "quic_connection_pool_workload_hint".to_string(),
        serde_json::to_value(quic_connection_pool_workload_hint(&state.args))
            .expect("quic pool workload hint serializable"),
    );
    status.insert(
        "quic_connection_pool_reason".to_string(),
        quic_connection_pool_reason(&state.args).into(),
    );
    status.insert(
        "quic_connection_pool_mode".to_string(),
        quic_connection_pool_mode(state.args.transport_pool_size.max(1)).into(),
    );
    status.insert(
        "quic_connection_pool_selection_policy".to_string(),
        QUIC_CONNECTION_POOL_SELECTION_POLICY.into(),
    );
    status.insert(
        "active_quic_connections".to_string(),
        active_quic_connections.into(),
    );
    status.insert(
        "quic_connections".to_string(),
        serde_json::Value::Array(quic_connections.clone()),
    );
    status.insert("active_quic_flows".to_string(), active_quic_flows.into());
    status.insert(
        "quic_stream_open_samples".to_string(),
        quic_stream_open_samples.into(),
    );
    status.insert(
        "last_quic_stream_open_latency_ms".to_string(),
        serde_json::to_value(last_sampled_u64(
            quic_stream_open_samples,
            state
                .last_quic_stream_open_latency_ms
                .load(Ordering::Relaxed),
        ))
        .expect("stream open latency serializable"),
    );
    status.insert(
        "max_quic_stream_open_latency_ms".to_string(),
        serde_json::to_value(last_sampled_u64(
            quic_stream_open_samples,
            state
                .max_quic_stream_open_latency_ms
                .load(Ordering::Relaxed),
        ))
        .expect("max stream open latency serializable"),
    );
    status.insert(
        "quic_stream_open_failures".to_string(),
        quic_stream_open_failures.into(),
    );
    status.insert(
        "quic_header_write_samples".to_string(),
        state
            .quic_header_write_samples
            .load(Ordering::Relaxed)
            .into(),
    );
    status.insert(
        "last_quic_header_write_latency_ms".to_string(),
        serde_json::to_value(last_sampled_u64(
            state.quic_header_write_samples.load(Ordering::Relaxed),
            state
                .last_quic_header_write_latency_ms
                .load(Ordering::Relaxed),
        ))
        .expect("header write latency serializable"),
    );
    status.insert(
        "max_quic_header_write_latency_ms".to_string(),
        serde_json::to_value(last_sampled_u64(
            state.quic_header_write_samples.load(Ordering::Relaxed),
            state
                .max_quic_header_write_latency_ms
                .load(Ordering::Relaxed),
        ))
        .expect("max header write latency serializable"),
    );
    status.insert(
        "quic_header_write_failures".to_string(),
        state
            .quic_header_write_failures
            .load(Ordering::Relaxed)
            .into(),
    );
    status.insert(
        "quic_backpressure_timeouts".to_string(),
        state
            .quic_backpressure_timeouts
            .load(Ordering::Relaxed)
            .into(),
    );
    status.insert(
        "quic_flow_graceful_closes".to_string(),
        state
            .quic_flow_graceful_closes
            .load(Ordering::Relaxed)
            .into(),
    );
    status.insert(
        "quic_flow_resets".to_string(),
        state.quic_flow_resets.load(Ordering::Relaxed).into(),
    );
    status.insert(
        "quic_flow_first_byte_samples".to_string(),
        first_byte_samples.into(),
    );
    status.insert(
        "last_quic_flow_first_byte_latency_ms".to_string(),
        serde_json::to_value(last_quic_flow_first_byte_latency_ms)
            .expect("first byte latency serializable"),
    );
    status.insert(
        "max_quic_flow_first_byte_latency_ms".to_string(),
        serde_json::to_value(last_sampled_u64(
            first_byte_samples,
            state
                .max_quic_flow_first_byte_latency_ms
                .load(Ordering::Relaxed),
        ))
        .expect("max first byte latency serializable"),
    );
    status.insert(
        "quic_copy_duration_samples".to_string(),
        state
            .quic_copy_duration_samples
            .load(Ordering::Relaxed)
            .into(),
    );
    status.insert(
        "last_quic_copy_duration_ms".to_string(),
        serde_json::to_value(last_sampled_u64(
            state.quic_copy_duration_samples.load(Ordering::Relaxed),
            state.last_quic_copy_duration_ms.load(Ordering::Relaxed),
        ))
        .expect("copy duration serializable"),
    );
    status.insert(
        "max_quic_copy_duration_ms".to_string(),
        serde_json::to_value(last_sampled_u64(
            state.quic_copy_duration_samples.load(Ordering::Relaxed),
            state.max_quic_copy_duration_ms.load(Ordering::Relaxed),
        ))
        .expect("max copy duration serializable"),
    );
    status.insert(
        "quic_copy_failures".to_string(),
        state.quic_copy_failures.load(Ordering::Relaxed).into(),
    );
    status.insert(
        "last_quic_copy_client_to_remote_bytes".to_string(),
        serde_json::to_value(last_sampled_u64(
            state.quic_copy_duration_samples.load(Ordering::Relaxed),
            state
                .last_quic_copy_client_to_remote_bytes
                .load(Ordering::Relaxed),
        ))
        .expect("copy bytes serializable"),
    );
    status.insert(
        "last_quic_copy_remote_to_client_bytes".to_string(),
        serde_json::to_value(last_sampled_u64(
            state.quic_copy_duration_samples.load(Ordering::Relaxed),
            state
                .last_quic_copy_remote_to_client_bytes
                .load(Ordering::Relaxed),
        ))
        .expect("copy bytes serializable"),
    );
    status.insert(
        "max_quic_copy_client_to_remote_bytes".to_string(),
        serde_json::to_value(last_sampled_u64(
            state.quic_copy_duration_samples.load(Ordering::Relaxed),
            state
                .max_quic_copy_client_to_remote_bytes
                .load(Ordering::Relaxed),
        ))
        .expect("max copy bytes serializable"),
    );
    status.insert(
        "max_quic_copy_remote_to_client_bytes".to_string(),
        serde_json::to_value(last_sampled_u64(
            state.quic_copy_duration_samples.load(Ordering::Relaxed),
            state
                .max_quic_copy_remote_to_client_bytes
                .load(Ordering::Relaxed),
        ))
        .expect("max copy bytes serializable"),
    );
    status.insert(
        "active_tcp".to_string(),
        state.active_tcp.load(Ordering::Relaxed).into(),
    );
    status.insert(
        "total_tcp".to_string(),
        state.total_tcp.load(Ordering::Relaxed).into(),
    );
    status.insert(
        "tcp_open_attempts".to_string(),
        state.tcp_open_attempts.load(Ordering::Relaxed).into(),
    );
    status.insert(
        "tcp_open_successes".to_string(),
        state.tcp_open_successes.load(Ordering::Relaxed).into(),
    );
    status.insert(
        "tcp_open_failures".to_string(),
        state.tcp_open_failures.load(Ordering::Relaxed).into(),
    );
    status.insert(
        "last_tcp_open_latency_ms".to_string(),
        serde_json::to_value(last_sampled_u64(
            state.tcp_open_attempts.load(Ordering::Relaxed),
            state.last_tcp_open_latency_ms.load(Ordering::Relaxed),
        ))
        .expect("tcp open latency serializable"),
    );
    status.insert(
        "bytes_client_to_remote".to_string(),
        bytes_client_to_remote.into(),
    );
    status.insert(
        "bytes_remote_to_client".to_string(),
        bytes_remote_to_client.into(),
    );
    status.insert("control_state".to_string(), control_state.into());
    status.insert("control_degraded".to_string(), control_degraded.into());
    status.insert(
        "control_pings_sent".to_string(),
        state.control_pings_sent.load(Ordering::Relaxed).into(),
    );
    status.insert(
        "control_pongs_received".to_string(),
        state.control_pongs_received.load(Ordering::Relaxed).into(),
    );
    status.insert(
        "last_control_pong_latency_ms".to_string(),
        serde_json::to_value(last_sampled_u64(
            state.control_pongs_received.load(Ordering::Relaxed),
            state.last_control_pong_latency_ms.load(Ordering::Relaxed),
        ))
        .expect("control pong latency serializable"),
    );
    status.insert(
        "control_keepalive_interval_secs".to_string(),
        CONTROL_KEEPALIVE_INTERVAL.as_secs().into(),
    );
    status.insert(
        "control_keepalive_timeout_secs".to_string(),
        CONTROL_KEEPALIVE_TIMEOUT.as_secs().into(),
    );
    status.insert(
        "last_control_error".to_string(),
        serde_json::to_value(last_control_error.clone()).expect("control error serializable"),
    );
    status.insert(
        "read_buffer_size".to_string(),
        QUIC_NATIVE_COPY_BUFFER_SIZE.into(),
    );
    status.insert(
        "quic_copy_buffer_size".to_string(),
        QUIC_NATIVE_COPY_BUFFER_SIZE.into(),
    );
    status.insert(
        "quic_stream_open_timeout_secs".to_string(),
        state.args.connect_timeout_secs.max(1).into(),
    );
    status.insert(
        "quic_first_byte_timeout_secs".to_string(),
        QUIC_NATIVE_FIRST_BYTE_TIMEOUT.as_secs().into(),
    );
    status.insert(
        "quic_backpressure_timeout_secs".to_string(),
        QUIC_NATIVE_BACKPRESSURE_TIMEOUT.as_secs().into(),
    );
    status.insert(
        "write_batch_limit".to_string(),
        crate::protocol::FRAME_WRITE_BATCH_LIMIT.into(),
    );
    status.insert(
        "frame_channel_capacity".to_string(),
        crate::protocol::FRAME_CHANNEL_CAPACITY.into(),
    );
    status.insert(
        "quic_receive_window".to_string(),
        quic_options.receive_window.into(),
    );
    status.insert(
        "quic_stream_receive_window".to_string(),
        quic_options.stream_receive_window.into(),
    );
    status.insert(
        "quic_max_bidi_streams".to_string(),
        quic_options.max_bidi_streams.into(),
    );
    status.insert(
        "quic_keep_alive_interval_secs".to_string(),
        quic_options.keep_alive_interval_secs.into(),
    );
    status.insert(
        "quic_idle_timeout_secs".to_string(),
        quic_options.idle_timeout_secs.into(),
    );
    status.insert(
        "quic_runtime".to_string(),
        serde_json::to_value(quic_runtime.clone()).expect("quic runtime diagnostics serializable"),
    );
    status.insert(
        "quic_udp_runtime".to_string(),
        peer_transport::QUIC_UDP_RUNTIME.into(),
    );
    status.insert("quic_udp_gso".to_string(), serde_json::Value::Null);
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
        serde_json::json!({
            "enabled": true,
            "profile_scope": "quic-native-route-status",
            "connected": !state.shutdown.load(Ordering::Relaxed),
            "mode": "native-per-flow",
            "route_id": state.route_id.clone(),
            "active_connections": active_quic_connections,
            "active_flows": active_quic_flows,
            "pool": {
                "size": state.args.transport_pool_size.max(1),
                "active_connections": active_quic_connections,
                "policy": quic_connection_pool_policy(&state.args),
                "workload_hint": quic_connection_pool_workload_hint(&state.args),
                "reason": quic_connection_pool_reason(&state.args),
                "mode": quic_connection_pool_mode(state.args.transport_pool_size.max(1)),
                "selection_policy": QUIC_CONNECTION_POOL_SELECTION_POLICY,
            },
            "transport": quic_runtime.transport_options,
            "runtime": quic_runtime.clone(),
            "udp": {
                "runtime": peer_transport::QUIC_UDP_RUNTIME,
                "gso": serde_json::Value::Null,
                "gso_source": peer_transport::QUIC_UDP_GSO_SOURCE,
                "packetization": peer_transport::QUIC_PACKETIZATION,
                "max_datagram_size": serde_json::Value::Null,
                "max_datagram_size_source": "unavailable: Quinn endpoint API is not exposed through ssh_proxy status",
                "packet_loss": serde_json::Value::Null,
                "packet_loss_source": "unavailable: Quinn connection loss counters are not exposed through ssh_proxy status",
                "congestion_signal": serde_json::Value::Null,
                "congestion_signal_source": "unavailable: Quinn congestion counters are not exposed through ssh_proxy status",
            },
            "connections": quic_connections.clone(),
            "flow": {
                "stream_open_samples": quic_stream_open_samples,
                "stream_open_failures": quic_stream_open_failures,
                "last_stream_open_latency_ms": serde_json::to_value(last_sampled_u64(
                    quic_stream_open_samples,
                    state.last_quic_stream_open_latency_ms.load(Ordering::Relaxed),
                )).expect("stream open latency serializable"),
                "max_stream_open_latency_ms": serde_json::to_value(last_sampled_u64(
                    quic_stream_open_samples,
                    state.max_quic_stream_open_latency_ms.load(Ordering::Relaxed),
                )).expect("max stream open latency serializable"),
                "header_write_samples": quic_header_write_samples,
                "header_write_failures": quic_header_write_failures,
                "last_header_write_latency_ms": serde_json::to_value(last_sampled_u64(
                    quic_header_write_samples,
                    state.last_quic_header_write_latency_ms.load(Ordering::Relaxed),
                )).expect("header write latency serializable"),
                "max_header_write_latency_ms": serde_json::to_value(last_sampled_u64(
                    quic_header_write_samples,
                    state.max_quic_header_write_latency_ms.load(Ordering::Relaxed),
                )).expect("max header write latency serializable"),
                "first_byte_samples": first_byte_samples,
                "last_first_byte_latency_ms": serde_json::to_value(last_quic_flow_first_byte_latency_ms).expect("first byte latency serializable"),
                "max_first_byte_latency_ms": serde_json::to_value(max_quic_flow_first_byte_latency_ms).expect("max first byte latency serializable"),
                "copy_duration_samples": quic_copy_duration_samples,
                "last_copy_duration_ms": serde_json::to_value(last_quic_copy_duration_ms).expect("copy duration serializable"),
                "max_copy_duration_ms": serde_json::to_value(max_quic_copy_duration_ms).expect("max copy duration serializable"),
                "copy_failures": quic_copy_failures,
                "last_copy_client_to_remote_bytes": serde_json::to_value(last_quic_copy_client_to_remote_bytes).expect("copy bytes serializable"),
                "last_copy_remote_to_client_bytes": serde_json::to_value(last_quic_copy_remote_to_client_bytes).expect("copy bytes serializable"),
                "max_copy_client_to_remote_bytes": serde_json::to_value(max_quic_copy_client_to_remote_bytes).expect("max copy bytes serializable"),
                "max_copy_remote_to_client_bytes": serde_json::to_value(max_quic_copy_remote_to_client_bytes).expect("max copy bytes serializable"),
                "backpressure_timeouts": quic_backpressure_timeouts,
                "graceful_closes": quic_flow_graceful_closes,
                "resets": quic_flow_resets,
            },
            "control": {
                "state": control_state,
                "degraded": control_degraded,
                "pings_sent": control_pings_sent,
                "pongs_received": control_pongs_received,
                "last_pong_latency_ms": serde_json::to_value(last_control_pong_latency_ms).expect("control pong latency serializable"),
                "last_error": last_control_error.clone(),
            },
            "signals": {
                "next_bottleneck": quic_profile_next_bottleneck(
                    control_degraded,
                    quic_backpressure_timeouts,
                    quic_copy_failures,
                    quic_copy_duration_samples,
                    quic_stream_open_failures,
                    quic_header_write_failures,
                ),
                "window_sizing": {
                    "suspected": quic_stream_open_failures > 0 || quic_header_write_failures > 0,
                    "evidence": if quic_stream_open_failures > 0 || quic_header_write_failures > 0 {
                        "stream or header opens are failing"
                    } else if quic_stream_open_samples == 0 {
                        "no successful QUIC stream opens recorded"
                    } else {
                        "stream opens are currently healthy"
                    },
                },
                "udp_path": {
                    "suspected": quic_runtime.udp_gso.is_none(),
                    "evidence": quic_runtime.udp_gso_source,
                },
                "application_copy": {
                    "suspected": quic_copy_failures > 0 || quic_copy_duration_samples > 0,
                    "evidence": if quic_copy_failures > 0 {
                        "copy failures were recorded"
                    } else if quic_copy_duration_samples > 0 {
                        "copy duration samples are present"
                    } else {
                        "no copy samples recorded"
                    },
                },
                "slow_consumers": {
                    "suspected": quic_backpressure_timeouts > 0,
                    "evidence": if quic_backpressure_timeouts > 0 {
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
        }),
    );
    status.insert(
        "last_quic_flow_close_reason".to_string(),
        serde_json::to_value(last_quic_flow_close_reason.clone())
            .expect("flow close reason serializable"),
    );
    status.insert(
        "last_error".to_string(),
        serde_json::to_value(last_error.clone()).expect("last error serializable"),
    );
    status.insert(
        "link".to_string(),
        serde_json::json!({
            "health": {
                "selected_protocol": "quic-native",
                "active_connections": active_quic_connections,
                "active_streams": active_quic_flows,
                "active_channels": active_quic_flows,
                "pool_size": state.args.transport_pool_size.max(1),
                "pool_policy": quic_connection_pool_policy(&state.args),
                "pool_workload_hint": quic_connection_pool_workload_hint(&state.args),
                "pool_reason": quic_connection_pool_reason(&state.args),
                "pool_mode": quic_connection_pool_mode(state.args.transport_pool_size.max(1)),
                "pool_selection_policy": QUIC_CONNECTION_POOL_SELECTION_POLICY,
                "open_attempts": quic_stream_open_samples + quic_stream_open_failures,
                "open_successes": quic_stream_open_samples,
                "open_failures": quic_stream_open_failures,
                "open_latency_ms": last_sampled_u64(
                    quic_stream_open_samples,
                    state.last_quic_stream_open_latency_ms.load(Ordering::Relaxed),
                ),
                "bytes_client_to_remote": bytes_client_to_remote,
                "bytes_remote_to_client": bytes_remote_to_client,
                "first_byte_samples": first_byte_samples,
                "first_byte_latency_ms": last_quic_flow_first_byte_latency_ms,
                "max_first_byte_latency_ms": max_quic_flow_first_byte_latency_ms,
                "last_close_reason": last_quic_flow_close_reason,
                "degraded_reason": last_control_error.clone().or(last_error),
                "control_health": control_state,
                "connected": !state.shutdown.load(Ordering::Relaxed),
            }
        }),
    );
    serde_json::Value::Object(status)
}
