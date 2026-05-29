use std::{sync::atomic::Ordering, time::Duration};

use crate::quic_native::{
    metrics::{duration_millis, record_latency_sample, update_max},
    pool::quic_worker_score,
};

use super::{ConnectionWorker, State};

impl ConnectionWorker {
    pub(in crate::quic_native::runtime) fn score(&self) -> u64 {
        quic_worker_score(
            self.active_quic_flows.load(Ordering::Relaxed),
            self.stream_open_failures.load(Ordering::Relaxed),
            self.last_stream_open_latency_ms.load(Ordering::Relaxed),
            self.quic_backpressure_timeouts.load(Ordering::Relaxed),
            self.quic_flow_resets.load(Ordering::Relaxed),
            self.control_degraded.load(Ordering::Relaxed),
        )
    }

    pub(in crate::quic_native::runtime) fn record_open_success(&self, latency: Duration) {
        self.opened_quic_flows.fetch_add(1, Ordering::Relaxed);
        self.last_stream_open_latency_ms
            .store(duration_millis(latency), Ordering::Relaxed);
    }

    pub(in crate::quic_native::runtime) fn record_open_failure(&self) {
        self.stream_open_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::quic_native::runtime) fn record_flow_opened(&self) {
        self.active_quic_flows.fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::quic_native::runtime) fn record_flow_closed(&self, reset: bool) {
        self.active_quic_flows
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_sub(1)
            })
            .ok();
        if reset {
            self.quic_flow_resets.fetch_add(1, Ordering::Relaxed);
        } else {
            self.quic_flow_graceful_closes
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_backpressure_timeout(&self) {
        self.quic_backpressure_timeouts
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::quic_native::runtime) async fn mark_control_degraded(
        &self,
        err: impl Into<String>,
    ) {
        self.control_degraded.store(true, Ordering::Relaxed);
        *self.last_control_error.lock().await = Some(err.into());
    }
}

impl State {
    pub fn record_tcp_open(&self) {
        self.total_tcp.fetch_add(1, Ordering::Relaxed);
        self.active_tcp.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_tcp_close(&self) {
        self.active_tcp
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_sub(1)
            })
            .ok();
    }

    pub fn record_tcp_open_attempt(&self) {
        self.tcp_open_attempts.fetch_add(1, Ordering::Relaxed);
    }

    pub async fn record_tcp_open_success(&self) {
        self.tcp_open_successes.fetch_add(1, Ordering::Relaxed);
        *self.last_error.lock().await = None;
    }

    pub async fn record_tcp_open_failure(&self, err: impl Into<String>) {
        self.tcp_open_failures.fetch_add(1, Ordering::Relaxed);
        *self.last_error.lock().await = Some(err.into());
    }

    pub fn record_tcp_open_latency(&self, latency: Duration) {
        self.last_tcp_open_latency_ms
            .store(duration_millis(latency), Ordering::Relaxed);
    }

    pub fn record_client_to_remote_bytes(&self, bytes: usize) {
        self.bytes_client_to_remote
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    pub fn record_remote_to_client_bytes(&self, bytes: usize) {
        self.bytes_remote_to_client
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    pub(in crate::quic_native::runtime) async fn record_quic_flow_close(
        &self,
        reason: impl Into<String>,
        reset: bool,
    ) {
        if reset {
            self.quic_flow_resets.fetch_add(1, Ordering::Relaxed);
        } else {
            self.quic_flow_graceful_closes
                .fetch_add(1, Ordering::Relaxed);
        }
        *self.last_quic_flow_close_reason.lock().await = Some(reason.into());
    }

    pub(in crate::quic_native::runtime) fn record_quic_flow_drop(&self) {
        self.quic_flow_resets.fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::quic_native::runtime) fn record_quic_flow_first_byte_latency(
        &self,
        latency: Duration,
    ) {
        record_latency_sample(
            &self.quic_flow_first_byte_samples,
            &self.last_quic_flow_first_byte_latency_ms,
            &self.max_quic_flow_first_byte_latency_ms,
            latency,
        );
    }

    pub(in crate::quic_native::runtime) fn record_quic_stream_open_latency(
        &self,
        latency: Duration,
    ) {
        record_latency_sample(
            &self.quic_stream_open_samples,
            &self.last_quic_stream_open_latency_ms,
            &self.max_quic_stream_open_latency_ms,
            latency,
        );
    }

    pub(in crate::quic_native::runtime) fn record_quic_header_write_latency(
        &self,
        latency: Duration,
    ) {
        record_latency_sample(
            &self.quic_header_write_samples,
            &self.last_quic_header_write_latency_ms,
            &self.max_quic_header_write_latency_ms,
            latency,
        );
    }

    pub fn record_quic_copy(
        &self,
        duration: Duration,
        client_to_remote_bytes: u64,
        remote_to_client_bytes: u64,
    ) {
        record_latency_sample(
            &self.quic_copy_duration_samples,
            &self.last_quic_copy_duration_ms,
            &self.max_quic_copy_duration_ms,
            duration,
        );
        self.last_quic_copy_client_to_remote_bytes
            .store(client_to_remote_bytes, Ordering::Relaxed);
        self.last_quic_copy_remote_to_client_bytes
            .store(remote_to_client_bytes, Ordering::Relaxed);
        update_max(
            &self.max_quic_copy_client_to_remote_bytes,
            client_to_remote_bytes,
        );
        update_max(
            &self.max_quic_copy_remote_to_client_bytes,
            remote_to_client_bytes,
        );
    }

    pub fn record_quic_copy_failure(&self) {
        self.quic_copy_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_quic_backpressure_timeout(&self) {
        self.quic_backpressure_timeouts
            .fetch_add(1, Ordering::Relaxed);
    }
}
