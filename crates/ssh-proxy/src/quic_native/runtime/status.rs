use anyhow::Result;

use crate::{
    peer_transport,
    quic_native::pool::{
        QUIC_CONNECTION_POOL_SELECTION_POLICY, quic_connection_pool_mode,
        quic_connection_pool_policy, quic_connection_pool_reason,
        quic_connection_pool_workload_hint,
    },
};

use super::{
    CONTROL_KEEPALIVE_INTERVAL, CONTROL_KEEPALIVE_TIMEOUT, QUIC_NATIVE_BACKPRESSURE_TIMEOUT,
    QUIC_NATIVE_COPY_BUFFER_SIZE, QUIC_NATIVE_FIRST_BYTE_TIMEOUT, State,
};

mod profile;
mod render;
mod snapshot;

use snapshot::QuicStatusSnapshot;

pub(super) async fn status_json(state: &State) -> Result<String> {
    render::pretty_json_line(status_value(state).await)
}

pub(super) async fn status_value(state: &State) -> serde_json::Value {
    let snapshot = QuicStatusSnapshot::collect(state).await;
    let quic_connections = snapshot.quic_connections.clone();
    let quic_options = snapshot.quic_options;
    let quic_runtime = snapshot.quic_runtime.clone();
    let active_quic_connections = snapshot.active_quic_connections;
    let active_quic_flows = snapshot.active_quic_flows;
    let quic_stream_open_samples = snapshot.quic_stream_open_samples;
    let last_quic_stream_open_latency_ms = snapshot.last_quic_stream_open_latency_ms;
    let max_quic_stream_open_latency_ms = snapshot.max_quic_stream_open_latency_ms;
    let quic_stream_open_failures = snapshot.quic_stream_open_failures;
    let quic_header_write_samples = snapshot.quic_header_write_samples;
    let last_quic_header_write_latency_ms = snapshot.last_quic_header_write_latency_ms;
    let max_quic_header_write_latency_ms = snapshot.max_quic_header_write_latency_ms;
    let quic_header_write_failures = snapshot.quic_header_write_failures;
    let quic_backpressure_timeouts = snapshot.quic_backpressure_timeouts;
    let quic_flow_graceful_closes = snapshot.quic_flow_graceful_closes;
    let quic_flow_resets = snapshot.quic_flow_resets;
    let first_byte_samples = snapshot.first_byte_samples;
    let last_quic_flow_first_byte_latency_ms = snapshot.last_quic_flow_first_byte_latency_ms;
    let max_quic_flow_first_byte_latency_ms = snapshot.max_quic_flow_first_byte_latency_ms;
    let quic_copy_duration_samples = snapshot.quic_copy_duration_samples;
    let last_quic_copy_duration_ms = snapshot.last_quic_copy_duration_ms;
    let max_quic_copy_duration_ms = snapshot.max_quic_copy_duration_ms;
    let quic_copy_failures = snapshot.quic_copy_failures;
    let last_quic_copy_client_to_remote_bytes = snapshot.last_quic_copy_client_to_remote_bytes;
    let last_quic_copy_remote_to_client_bytes = snapshot.last_quic_copy_remote_to_client_bytes;
    let max_quic_copy_client_to_remote_bytes = snapshot.max_quic_copy_client_to_remote_bytes;
    let max_quic_copy_remote_to_client_bytes = snapshot.max_quic_copy_remote_to_client_bytes;
    let bytes_client_to_remote = snapshot.bytes_client_to_remote;
    let bytes_remote_to_client = snapshot.bytes_remote_to_client;
    let control_degraded = snapshot.control_degraded;
    let control_state = snapshot.control_state;
    let control_pings_sent = snapshot.control_pings_sent;
    let control_pongs_received = snapshot.control_pongs_received;
    let last_control_pong_latency_ms = snapshot.last_control_pong_latency_ms;
    let last_control_error = snapshot.last_control_error.clone();
    let last_quic_flow_close_reason = snapshot.last_quic_flow_close_reason.clone();
    let last_error = snapshot.last_error.clone();
    let mut status = serde_json::Map::new();
    status.insert("connected".to_string(), snapshot.connected.into());
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
    status.insert("uptime_secs".to_string(), snapshot.uptime_secs.into());
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
        serde_json::to_value(last_quic_stream_open_latency_ms)
            .expect("stream open latency serializable"),
    );
    status.insert(
        "max_quic_stream_open_latency_ms".to_string(),
        serde_json::to_value(max_quic_stream_open_latency_ms)
            .expect("max stream open latency serializable"),
    );
    status.insert(
        "quic_stream_open_failures".to_string(),
        quic_stream_open_failures.into(),
    );
    status.insert(
        "quic_header_write_samples".to_string(),
        quic_header_write_samples.into(),
    );
    status.insert(
        "last_quic_header_write_latency_ms".to_string(),
        serde_json::to_value(last_quic_header_write_latency_ms)
            .expect("header write latency serializable"),
    );
    status.insert(
        "max_quic_header_write_latency_ms".to_string(),
        serde_json::to_value(max_quic_header_write_latency_ms)
            .expect("max header write latency serializable"),
    );
    status.insert(
        "quic_header_write_failures".to_string(),
        quic_header_write_failures.into(),
    );
    status.insert(
        "quic_backpressure_timeouts".to_string(),
        quic_backpressure_timeouts.into(),
    );
    status.insert(
        "quic_flow_graceful_closes".to_string(),
        quic_flow_graceful_closes.into(),
    );
    status.insert("quic_flow_resets".to_string(), quic_flow_resets.into());
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
        serde_json::to_value(max_quic_flow_first_byte_latency_ms)
            .expect("max first byte latency serializable"),
    );
    status.insert(
        "quic_copy_duration_samples".to_string(),
        quic_copy_duration_samples.into(),
    );
    status.insert(
        "last_quic_copy_duration_ms".to_string(),
        serde_json::to_value(last_quic_copy_duration_ms).expect("copy duration serializable"),
    );
    status.insert(
        "max_quic_copy_duration_ms".to_string(),
        serde_json::to_value(max_quic_copy_duration_ms).expect("max copy duration serializable"),
    );
    status.insert("quic_copy_failures".to_string(), quic_copy_failures.into());
    status.insert(
        "last_quic_copy_client_to_remote_bytes".to_string(),
        serde_json::to_value(last_quic_copy_client_to_remote_bytes)
            .expect("copy bytes serializable"),
    );
    status.insert(
        "last_quic_copy_remote_to_client_bytes".to_string(),
        serde_json::to_value(last_quic_copy_remote_to_client_bytes)
            .expect("copy bytes serializable"),
    );
    status.insert(
        "max_quic_copy_client_to_remote_bytes".to_string(),
        serde_json::to_value(max_quic_copy_client_to_remote_bytes)
            .expect("max copy bytes serializable"),
    );
    status.insert(
        "max_quic_copy_remote_to_client_bytes".to_string(),
        serde_json::to_value(max_quic_copy_remote_to_client_bytes)
            .expect("max copy bytes serializable"),
    );
    status.insert("active_tcp".to_string(), snapshot.active_tcp.into());
    status.insert("total_tcp".to_string(), snapshot.total_tcp.into());
    status.insert(
        "tcp_open_attempts".to_string(),
        snapshot.tcp_open_attempts.into(),
    );
    status.insert(
        "tcp_open_successes".to_string(),
        snapshot.tcp_open_successes.into(),
    );
    status.insert(
        "tcp_open_failures".to_string(),
        snapshot.tcp_open_failures.into(),
    );
    status.insert(
        "last_tcp_open_latency_ms".to_string(),
        serde_json::to_value(snapshot.last_tcp_open_latency_ms)
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
    status.insert("control_pings_sent".to_string(), control_pings_sent.into());
    status.insert(
        "control_pongs_received".to_string(),
        control_pongs_received.into(),
    );
    status.insert(
        "last_control_pong_latency_ms".to_string(),
        serde_json::to_value(last_control_pong_latency_ms)
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
        profile::quic_profile_value(state, &snapshot),
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
    status.insert("link".to_string(), profile::link_value(state, &snapshot));
    serde_json::Value::Object(status)
}
