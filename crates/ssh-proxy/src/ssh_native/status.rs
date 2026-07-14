use std::sync::atomic::Ordering;

use anyhow::Result;
use serde_json::{Value, json};

use super::{
    SSH_SESSION_GROWTH_MIN_ACTIVE_CHANNELS, State, average_sampled_u64, duration_millis,
    last_sampled_u64, latency_percentile, workload_hint_name,
};
use crate::protocol;

pub(super) async fn status_value(state: &State) -> Value {
    let sessions = state.sessions.lock().await;
    let now_ms = duration_millis(state.started.elapsed());
    let active_ssh_channels = state.active_ssh_channels.load(Ordering::Relaxed);
    let ssh_channel_open_attempts = state.ssh_channel_open_attempts.load(Ordering::Relaxed);
    let ssh_channel_open_failures = state.ssh_channel_open_failures.load(Ordering::Relaxed);
    let bytes_client_to_remote = state.bytes_client_to_remote.load(Ordering::Relaxed);
    let bytes_remote_to_client = state.bytes_remote_to_client.load(Ordering::Relaxed);
    let first_byte_samples = state.first_byte_samples.load(Ordering::Relaxed);
    let last_first_byte_latency_ms = last_sampled_u64(
        first_byte_samples,
        state.last_first_byte_latency_ms.load(Ordering::Relaxed),
    );
    let last_close_reason = state
        .last_close_reason
        .lock()
        .ok()
        .and_then(|reason| reason.clone());
    let last_error = state.last_error.lock().await.clone();
    let workers = sessions
        .iter()
        .map(|session| {
            let score_components = state.session_score_components(session, now_ms);
            json!({
                "session_id": session.id,
                "active_channels": session.active_channels.load(Ordering::Relaxed),
                "opened_channels": session.opened_channels.load(Ordering::Relaxed),
                "open_failures": session.open_failures.load(Ordering::Relaxed),
                "last_open_latency_ms": last_sampled_u64(
                    session.opened_channels.load(Ordering::Relaxed),
                    session.last_open_latency_ms.load(Ordering::Relaxed),
                ),
                "last_failure_age_ms": session.failure_age_ms(now_ms),
                "bytes_client_to_remote": session.bytes_client_to_remote.load(Ordering::Relaxed),
                "bytes_remote_to_client": session.bytes_remote_to_client.load(Ordering::Relaxed),
                "first_byte_samples": session.first_byte_samples.load(Ordering::Relaxed),
                "avg_first_byte_latency_ms": average_sampled_u64(
                    session.first_byte_samples.load(Ordering::Relaxed),
                    session.first_byte_latency_total_ms.load(Ordering::Relaxed),
                ),
                "last_first_byte_latency_ms": last_sampled_u64(
                    session.first_byte_samples.load(Ordering::Relaxed),
                    session.last_first_byte_latency_ms.load(Ordering::Relaxed),
                ),
                "max_first_byte_latency_ms": last_sampled_u64(
                    session.first_byte_samples.load(Ordering::Relaxed),
                    session.max_first_byte_latency_ms.load(Ordering::Relaxed),
                ),
                "p50_first_byte_latency_ms": latency_percentile(
                    &session.first_byte_latency_buckets,
                    session.first_byte_samples.load(Ordering::Relaxed),
                    50,
                ),
                "p95_first_byte_latency_ms": latency_percentile(
                    &session.first_byte_latency_buckets,
                    session.first_byte_samples.load(Ordering::Relaxed),
                    95,
                ),
                "graceful_closes": session.graceful_closes.load(Ordering::Relaxed),
                "error_closes": session.error_closes.load(Ordering::Relaxed),
                "last_close_reason": session.last_close_reason.lock().ok().and_then(|reason| reason.clone()),
                "last_channel_queue_depth": session.last_channel_queue_depth.load(Ordering::Relaxed),
                "max_channel_queue_depth": session.max_channel_queue_depth.load(Ordering::Relaxed),
                "score": score_components.score(),
                "score_components": score_components.to_json(),
            })
        })
        .collect::<Vec<_>>();
    let mut status = json!({
        "connected": !sessions.is_empty(),
        "selected_protocol": "ssh-native",
        "ssh_mode": "native-direct-tcpip",
        "ssh_session_pool_size": state.pool_size,
        "ssh_session_pool_source": state.args.ssh_session_pool_source.as_deref().unwrap_or("implicit"),
        "ssh_session_pool_reason": state.args.ssh_session_pool_reason.as_deref().unwrap_or_else(|| {
            if state.args.tcp_target.is_some() {
                "implicit ssh-native single-session default for fixed --tcp-target routes"
            } else {
                "implicit ssh-native two-session default for multi-flow SOCKS/HTTP proxy routes"
            }
        }),
        "ssh_session_pool_warning": state.args.ssh_session_pool_warning.as_deref(),
        "ssh_session_growth_active_threshold": SSH_SESSION_GROWTH_MIN_ACTIVE_CHANNELS,
        "ssh_session_growth_events": state.session_growth_events.load(Ordering::Relaxed),
        "ssh_session_growth_suppressed": state.session_growth_suppressed.load(Ordering::Relaxed),
        "active_ssh_sessions": sessions.len(),
        "active_ssh_channels": state.active_ssh_channels.load(Ordering::Relaxed),
        "ssh_session_connect_attempts": state.ssh_session_connect_attempts.load(Ordering::Relaxed),
        "ssh_session_connect_failures": state.ssh_session_connect_failures.load(Ordering::Relaxed),
        "ssh_channel_open_attempts": ssh_channel_open_attempts,
        "ssh_channel_open_failures": ssh_channel_open_failures,
        "workers": workers,
        "uptime_secs": state.started.elapsed().as_secs(),
        "active_tcp": state.active_tcp.load(Ordering::Relaxed),
        "total_tcp": state.total_tcp.load(Ordering::Relaxed),
        "tcp_open_attempts": state.tcp_open_attempts.load(Ordering::Relaxed),
        "tcp_open_successes": state.tcp_open_successes.load(Ordering::Relaxed),
        "tcp_open_failures": state.tcp_open_failures.load(Ordering::Relaxed),
        "last_tcp_open_latency_ms": last_sampled_u64(
            state.tcp_open_attempts.load(Ordering::Relaxed),
            state.last_tcp_open_latency_ms.load(Ordering::Relaxed),
        ),
        "bytes_client_to_remote": bytes_client_to_remote,
        "bytes_remote_to_client": bytes_remote_to_client,
        "first_byte_samples": first_byte_samples,
        "avg_first_byte_latency_ms": average_sampled_u64(
            first_byte_samples,
            state.first_byte_latency_total_ms.load(Ordering::Relaxed),
        ),
        "last_first_byte_latency_ms": last_first_byte_latency_ms,
        "max_first_byte_latency_ms": last_sampled_u64(
            first_byte_samples,
            state.max_first_byte_latency_ms.load(Ordering::Relaxed),
        ),
        "p50_first_byte_latency_ms": latency_percentile(
            &state.first_byte_latency_buckets,
            first_byte_samples,
            50,
        ),
        "p95_first_byte_latency_ms": latency_percentile(
            &state.first_byte_latency_buckets,
            first_byte_samples,
            95,
        ),
        "graceful_closes": state.graceful_closes.load(Ordering::Relaxed),
        "error_closes": state.error_closes.load(Ordering::Relaxed),
        "last_close_reason": last_close_reason,
        "last_channel_queue_depth": state.last_channel_queue_depth.load(Ordering::Relaxed),
        "max_channel_queue_depth": state.max_channel_queue_depth.load(Ordering::Relaxed),
        "read_buffer_size": protocol::TCP_DATA_CHUNK,
        "write_batch_limit": protocol::FRAME_WRITE_BATCH_LIMIT,
        "frame_channel_capacity": protocol::FRAME_CHANNEL_CAPACITY,
        "last_error": last_error,
    });
    if let Some(object) = status.as_object_mut() {
        object.insert(
            "ssh_session_scheduler".to_string(),
            json!({
                "policy": "warm-primary until active-channel pressure reaches growth threshold; otherwise choose the lowest scored healthy session",
                "growth_active_threshold": SSH_SESSION_GROWTH_MIN_ACTIVE_CHANNELS,
                "score_components": [
                    "active_channels",
                    "open_failures",
                    "last_open_latency_ms",
                    "first_byte_latency_ms",
                    "bytes_in_flight",
                    "recent_failure_penalty",
                    "error_closes",
                ],
            }),
        );
        object.insert(
            "link".to_string(),
            json!({
                "health": {
                    "selected_protocol": "ssh-native",
                    "active_connections": sessions.len(),
                    "active_streams": active_ssh_channels,
                    "active_channels": active_ssh_channels,
                    "pool_size": state.pool_size,
                    "pool_policy": state.args.pool_policy.as_deref(),
                    "pool_workload_hint": state.args.workload_hint.map(workload_hint_name),
                    "open_attempts": ssh_channel_open_attempts,
                    "open_successes": ssh_channel_open_attempts.saturating_sub(ssh_channel_open_failures),
                    "open_failures": ssh_channel_open_failures,
                    "open_latency_ms": last_sampled_u64(
                        ssh_channel_open_attempts,
                        state.last_tcp_open_latency_ms.load(Ordering::Relaxed),
                    ),
                    "bytes_client_to_remote": bytes_client_to_remote,
                    "bytes_remote_to_client": bytes_remote_to_client,
                    "first_byte_samples": first_byte_samples,
                    "first_byte_latency_ms": last_first_byte_latency_ms,
                    "avg_first_byte_latency_ms": average_sampled_u64(
                        first_byte_samples,
                        state.first_byte_latency_total_ms.load(Ordering::Relaxed),
                    ),
                    "max_first_byte_latency_ms": last_sampled_u64(
                        first_byte_samples,
                        state.max_first_byte_latency_ms.load(Ordering::Relaxed),
                    ),
                    "p50_first_byte_latency_ms": latency_percentile(
                        &state.first_byte_latency_buckets,
                        first_byte_samples,
                        50,
                    ),
                    "p95_first_byte_latency_ms": latency_percentile(
                        &state.first_byte_latency_buckets,
                        first_byte_samples,
                        95,
                    ),
                    "last_close_reason": last_close_reason,
                    "degraded_reason": last_error,
                    "healthy_workers": sessions.len(),
                    "degraded_workers": if last_error.is_some() { 1 } else { 0 },
                    "reconnecting_workers": 0,
                    "control_health": if sessions.is_empty() {
                        "disconnected"
                    } else if last_error.is_some() {
                        "degraded"
                    } else {
                        "healthy"
                    },
                    "connected": !sessions.is_empty(),
                }
            }),
        );
    }
    status
}

pub(super) async fn status_json(state: &State) -> Result<String> {
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&status_value(state).await)?
    ))
}
