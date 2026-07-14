use std::sync::atomic::Ordering;

use serde_json::Value;

use crate::quic_native::metrics::last_sampled_u64;

use super::State;

pub(super) async fn connection_status_values(state: &State) -> Vec<Value> {
    let mut values = Vec::with_capacity(state.workers.len());
    for worker in &state.workers {
        values.push(serde_json::json!({
            "worker_id": worker.id,
            "connected": !worker.control_degraded.load(Ordering::Relaxed),
            "uptime_secs": worker.started.elapsed().as_secs(),
            "active_quic_flows": worker.active_quic_flows.load(Ordering::Relaxed),
            "opened_quic_flows": worker.opened_quic_flows.load(Ordering::Relaxed),
            "stream_open_failures": worker.stream_open_failures.load(Ordering::Relaxed),
            "last_stream_open_latency_ms": last_sampled_u64(
                worker.opened_quic_flows.load(Ordering::Relaxed),
                worker.last_stream_open_latency_ms.load(Ordering::Relaxed),
            ),
            "bytes_client_to_remote": worker.bytes_client_to_remote.load(Ordering::Relaxed),
            "bytes_remote_to_client": worker.bytes_remote_to_client.load(Ordering::Relaxed),
            "quic_flow_graceful_closes": worker.quic_flow_graceful_closes.load(Ordering::Relaxed),
            "quic_flow_resets": worker.quic_flow_resets.load(Ordering::Relaxed),
            "quic_backpressure_timeouts": worker.quic_backpressure_timeouts.load(Ordering::Relaxed),
            "control_state": if worker.control_degraded.load(Ordering::Relaxed) {
                "degraded"
            } else {
                "healthy"
            },
            "control_degraded": worker.control_degraded.load(Ordering::Relaxed),
            "control_pings_sent": worker.control_pings_sent.load(Ordering::Relaxed),
            "control_pongs_received": worker.control_pongs_received.load(Ordering::Relaxed),
            "last_control_pong_latency_ms": last_sampled_u64(
                worker.control_pongs_received.load(Ordering::Relaxed),
                worker.last_control_pong_latency_ms.load(Ordering::Relaxed),
            ),
            "score_components": {
                "active_quic_flows": worker.active_quic_flows.load(Ordering::Relaxed),
                "stream_open_failures": worker.stream_open_failures.load(Ordering::Relaxed),
                "last_stream_open_latency_ms": worker.last_stream_open_latency_ms.load(Ordering::Relaxed),
                "quic_backpressure_timeouts": worker.quic_backpressure_timeouts.load(Ordering::Relaxed),
                "quic_flow_resets": worker.quic_flow_resets.load(Ordering::Relaxed),
                "control_degraded": worker.control_degraded.load(Ordering::Relaxed),
            },
            "last_control_error": worker.last_control_error.lock().await.clone(),
            "selection_score": worker.score(),
        }));
    }
    values
}
