use std::sync::atomic::Ordering;

use serde_json::Value;

use crate::{
    peer_transport,
    quic_native::{metrics::last_sampled_u64, runtime_config::quic_options_from_proxy_args},
};

use super::super::{State, metrics_snapshot::connection_status_values};

pub(super) struct QuicStatusSnapshot {
    pub quic_connections: Vec<Value>,
    pub quic_options: peer_transport::QuicTransportOptions,
    pub quic_runtime: peer_transport::QuicRuntimeDiagnostics,
    pub connected: bool,
    pub uptime_secs: u64,
    pub active_quic_connections: usize,
    pub active_quic_flows: u64,
    pub quic_stream_open_samples: u64,
    pub last_quic_stream_open_latency_ms: Option<u64>,
    pub max_quic_stream_open_latency_ms: Option<u64>,
    pub quic_stream_open_failures: u64,
    pub quic_header_write_samples: u64,
    pub last_quic_header_write_latency_ms: Option<u64>,
    pub max_quic_header_write_latency_ms: Option<u64>,
    pub quic_header_write_failures: u64,
    pub quic_backpressure_timeouts: u64,
    pub quic_flow_graceful_closes: u64,
    pub quic_flow_resets: u64,
    pub first_byte_samples: u64,
    pub last_quic_flow_first_byte_latency_ms: Option<u64>,
    pub max_quic_flow_first_byte_latency_ms: Option<u64>,
    pub quic_copy_duration_samples: u64,
    pub last_quic_copy_duration_ms: Option<u64>,
    pub max_quic_copy_duration_ms: Option<u64>,
    pub quic_copy_failures: u64,
    pub last_quic_copy_client_to_remote_bytes: Option<u64>,
    pub last_quic_copy_remote_to_client_bytes: Option<u64>,
    pub max_quic_copy_client_to_remote_bytes: Option<u64>,
    pub max_quic_copy_remote_to_client_bytes: Option<u64>,
    pub active_tcp: u64,
    pub total_tcp: u64,
    pub tcp_open_attempts: u64,
    pub tcp_open_successes: u64,
    pub tcp_open_failures: u64,
    pub last_tcp_open_latency_ms: Option<u64>,
    pub bytes_client_to_remote: u64,
    pub bytes_remote_to_client: u64,
    pub control_state: &'static str,
    pub control_degraded: bool,
    pub control_pings_sent: u64,
    pub control_pongs_received: u64,
    pub last_control_pong_latency_ms: Option<u64>,
    pub last_control_error: Option<String>,
    pub last_quic_flow_close_reason: Option<String>,
    pub last_error: Option<String>,
}

impl QuicStatusSnapshot {
    pub(super) async fn collect(state: &State) -> Self {
        let quic_connections = connection_status_values(state).await;
        let quic_options = quic_options_from_proxy_args(&state.args).unwrap_or_default();
        let quic_runtime = peer_transport::quic_runtime_diagnostics(quic_options);
        let active_quic_connections = state.workers.len();
        let active_quic_flows = state.active_quic_flows.load(Ordering::Relaxed);
        let quic_stream_open_samples = state.quic_stream_open_samples.load(Ordering::Relaxed);
        let quic_header_write_samples = state.quic_header_write_samples.load(Ordering::Relaxed);
        let first_byte_samples = state.quic_flow_first_byte_samples.load(Ordering::Relaxed);
        let quic_copy_duration_samples = state.quic_copy_duration_samples.load(Ordering::Relaxed);
        let control_degraded = state.control_degraded.load(Ordering::Relaxed);
        let control_pongs_received = state.control_pongs_received.load(Ordering::Relaxed);
        let tcp_open_attempts = state.tcp_open_attempts.load(Ordering::Relaxed);
        let connected = !state.shutdown.load(Ordering::Relaxed);

        Self {
            quic_connections,
            quic_options,
            quic_runtime,
            connected,
            uptime_secs: state.started.elapsed().as_secs(),
            active_quic_connections,
            active_quic_flows,
            quic_stream_open_samples,
            last_quic_stream_open_latency_ms: last_sampled_u64(
                quic_stream_open_samples,
                state
                    .last_quic_stream_open_latency_ms
                    .load(Ordering::Relaxed),
            ),
            max_quic_stream_open_latency_ms: last_sampled_u64(
                quic_stream_open_samples,
                state
                    .max_quic_stream_open_latency_ms
                    .load(Ordering::Relaxed),
            ),
            quic_stream_open_failures: state.quic_stream_open_failures.load(Ordering::Relaxed),
            quic_header_write_samples,
            last_quic_header_write_latency_ms: last_sampled_u64(
                quic_header_write_samples,
                state
                    .last_quic_header_write_latency_ms
                    .load(Ordering::Relaxed),
            ),
            max_quic_header_write_latency_ms: last_sampled_u64(
                quic_header_write_samples,
                state
                    .max_quic_header_write_latency_ms
                    .load(Ordering::Relaxed),
            ),
            quic_header_write_failures: state.quic_header_write_failures.load(Ordering::Relaxed),
            quic_backpressure_timeouts: state.quic_backpressure_timeouts.load(Ordering::Relaxed),
            quic_flow_graceful_closes: state.quic_flow_graceful_closes.load(Ordering::Relaxed),
            quic_flow_resets: state.quic_flow_resets.load(Ordering::Relaxed),
            first_byte_samples,
            last_quic_flow_first_byte_latency_ms: last_sampled_u64(
                first_byte_samples,
                state
                    .last_quic_flow_first_byte_latency_ms
                    .load(Ordering::Relaxed),
            ),
            max_quic_flow_first_byte_latency_ms: last_sampled_u64(
                first_byte_samples,
                state
                    .max_quic_flow_first_byte_latency_ms
                    .load(Ordering::Relaxed),
            ),
            quic_copy_duration_samples,
            last_quic_copy_duration_ms: last_sampled_u64(
                quic_copy_duration_samples,
                state.last_quic_copy_duration_ms.load(Ordering::Relaxed),
            ),
            max_quic_copy_duration_ms: last_sampled_u64(
                quic_copy_duration_samples,
                state.max_quic_copy_duration_ms.load(Ordering::Relaxed),
            ),
            quic_copy_failures: state.quic_copy_failures.load(Ordering::Relaxed),
            last_quic_copy_client_to_remote_bytes: last_sampled_u64(
                quic_copy_duration_samples,
                state
                    .last_quic_copy_client_to_remote_bytes
                    .load(Ordering::Relaxed),
            ),
            last_quic_copy_remote_to_client_bytes: last_sampled_u64(
                quic_copy_duration_samples,
                state
                    .last_quic_copy_remote_to_client_bytes
                    .load(Ordering::Relaxed),
            ),
            max_quic_copy_client_to_remote_bytes: last_sampled_u64(
                quic_copy_duration_samples,
                state
                    .max_quic_copy_client_to_remote_bytes
                    .load(Ordering::Relaxed),
            ),
            max_quic_copy_remote_to_client_bytes: last_sampled_u64(
                quic_copy_duration_samples,
                state
                    .max_quic_copy_remote_to_client_bytes
                    .load(Ordering::Relaxed),
            ),
            active_tcp: state.active_tcp.load(Ordering::Relaxed),
            total_tcp: state.total_tcp.load(Ordering::Relaxed),
            tcp_open_attempts,
            tcp_open_successes: state.tcp_open_successes.load(Ordering::Relaxed),
            tcp_open_failures: state.tcp_open_failures.load(Ordering::Relaxed),
            last_tcp_open_latency_ms: last_sampled_u64(
                tcp_open_attempts,
                state.last_tcp_open_latency_ms.load(Ordering::Relaxed),
            ),
            bytes_client_to_remote: state.bytes_client_to_remote.load(Ordering::Relaxed),
            bytes_remote_to_client: state.bytes_remote_to_client.load(Ordering::Relaxed),
            control_state: if control_degraded {
                "degraded"
            } else {
                "healthy"
            },
            control_degraded,
            control_pings_sent: state.control_pings_sent.load(Ordering::Relaxed),
            control_pongs_received,
            last_control_pong_latency_ms: last_sampled_u64(
                control_pongs_received,
                state.last_control_pong_latency_ms.load(Ordering::Relaxed),
            ),
            last_control_error: state.last_control_error.lock().await.clone(),
            last_quic_flow_close_reason: state.last_quic_flow_close_reason.lock().await.clone(),
            last_error: state.last_error.lock().await.clone(),
        }
    }
}
