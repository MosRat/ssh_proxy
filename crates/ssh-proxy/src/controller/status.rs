use std::{
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use anyhow::Result;
use ssh_proxy_transport::spx::SpxBridgeWorkerSnapshot;

use crate::{bridge, peer_transport, protocol};

use super::SharedState;

#[derive(Debug, Default)]
pub(super) struct BridgeWorkerCounters {
    pub(super) active_streams: AtomicU64,
    pub(super) bytes_client_to_remote: AtomicU64,
    pub(super) bytes_remote_to_client: AtomicU64,
}

#[derive(Debug, Clone)]
pub(super) struct BridgeWorkerState {
    pub(super) slot: usize,
    pub(super) connected: bool,
    pub(super) generation: u32,
    pub(super) connect_attempts: u64,
    pub(super) successful_connects: u64,
    pub(super) failed_connects: u64,
    pub(super) disconnects: u64,
    pub(super) retry_count: u64,
    pub(super) last_error: Option<String>,
    pub(super) selected_protocol: Option<String>,
    pub(super) last_successful_protocol: Option<String>,
    pub(super) last_event: Option<String>,
    pub(super) last_connected_at: Option<Instant>,
    pub(super) last_disconnected_at: Option<Instant>,
    pub(super) last_failed_at: Option<Instant>,
}

impl BridgeWorkerState {
    pub(super) fn new(slot: usize) -> Self {
        Self {
            slot,
            connected: false,
            generation: 0,
            connect_attempts: 0,
            successful_connects: 0,
            failed_connects: 0,
            disconnects: 0,
            retry_count: 0,
            last_error: None,
            selected_protocol: None,
            last_successful_protocol: None,
            last_event: None,
            last_connected_at: None,
            last_disconnected_at: None,
            last_failed_at: None,
        }
    }

    pub(super) fn connected(slot: usize, generation: u32) -> Self {
        let now = Instant::now();
        Self {
            slot,
            connected: true,
            generation,
            connect_attempts: 1,
            successful_connects: 1,
            failed_connects: 0,
            disconnects: 0,
            retry_count: 0,
            last_error: None,
            selected_protocol: Some(
                peer_transport::PeerProtocol::SshExec
                    .data_plane_label()
                    .to_string(),
            ),
            last_successful_protocol: Some(
                peer_transport::PeerProtocol::SshExec
                    .data_plane_label()
                    .to_string(),
            ),
            last_event: Some("bridge connected".to_string()),
            last_connected_at: Some(now),
            last_disconnected_at: None,
            last_failed_at: None,
        }
    }

    pub(super) fn snapshot(
        &self,
        now: Instant,
        counters: Option<&BridgeWorkerCounters>,
    ) -> SpxBridgeWorkerSnapshot {
        let state = self.health_state();
        let active_streams = counters
            .map(|counters| counters.active_streams.load(Ordering::Relaxed))
            .unwrap_or(0);
        let bytes_client_to_remote = counters
            .map(|counters| counters.bytes_client_to_remote.load(Ordering::Relaxed))
            .unwrap_or(0);
        let bytes_remote_to_client = counters
            .map(|counters| counters.bytes_remote_to_client.load(Ordering::Relaxed))
            .unwrap_or(0);
        SpxBridgeWorkerSnapshot {
            slot: self.slot,
            state,
            connected: self.connected,
            generation: self.generation,
            connect_attempts: self.connect_attempts,
            successful_connects: self.successful_connects,
            failed_connects: self.failed_connects,
            disconnects: self.disconnects,
            retry_count: self.retry_count,
            active_streams,
            bytes_client_to_remote,
            bytes_remote_to_client,
            last_error: self.last_error.clone(),
            degraded_reason: self.degraded_reason(),
            selected_protocol: self.selected_protocol.clone(),
            last_successful_protocol: self.last_successful_protocol.clone(),
            last_event: self.last_event.clone(),
            last_connected_ago_secs: self
                .last_connected_at
                .map(|instant| now.saturating_duration_since(instant).as_secs()),
            last_disconnected_ago_secs: self
                .last_disconnected_at
                .map(|instant| now.saturating_duration_since(instant).as_secs()),
            last_failure_ago_secs: self
                .last_failed_at
                .map(|instant| now.saturating_duration_since(instant).as_secs()),
        }
    }

    fn health_state(&self) -> String {
        if self.connected {
            "connected".to_string()
        } else if self.last_event.as_deref() == Some("connecting bridge") {
            "reconnecting".to_string()
        } else if self.last_error.is_some() {
            "degraded".to_string()
        } else if self.disconnects > 0 {
            "reconnecting".to_string()
        } else {
            "idle".to_string()
        }
    }

    fn degraded_reason(&self) -> Option<String> {
        if self.connected {
            None
        } else {
            self.last_error.clone()
        }
    }
}

pub(super) fn ensure_worker_slot(workers: &mut Vec<BridgeWorkerState>, slot: usize) {
    while workers.len() <= slot {
        let next = workers.len();
        workers.push(BridgeWorkerState::new(next));
    }
}

impl SharedState {
    async fn worker_snapshots(&self) -> Vec<SpxBridgeWorkerSnapshot> {
        let now = Instant::now();
        self.bridge_workers
            .read()
            .await
            .iter()
            .map(|worker| {
                worker.snapshot(
                    now,
                    self.bridge_worker_counters
                        .get(worker.slot)
                        .map(Arc::as_ref),
                )
            })
            .collect()
    }

    async fn active_selected_protocol(&self) -> Option<String> {
        let workers = self.bridge_workers.read().await;
        let mut selected = None::<String>;
        for worker in workers.iter().filter(|worker| worker.connected) {
            let Some(protocol) = worker.selected_protocol.clone() else {
                continue;
            };
            match &selected {
                Some(existing) if existing != &protocol => return Some("mixed".to_string()),
                Some(_) => {}
                None => selected = Some(protocol),
            }
        }
        selected
    }

    async fn bridge_metrics_snapshot(&self) -> bridge::BridgeMetricsSnapshot {
        let mut aggregate = bridge::BridgeMetricsSnapshot::default();
        for bridge in self
            .bridge_slots
            .read()
            .await
            .iter()
            .filter_map(Clone::clone)
        {
            aggregate.merge(&bridge.metrics_snapshot());
        }
        aggregate
    }

    pub async fn status_value(&self) -> serde_json::Value {
        let active_bridges = self.active_bridge_count().await;
        let workers = self.worker_snapshots().await;
        let selected_protocol = self.active_selected_protocol().await;
        let bridge_metrics = self.bridge_metrics_snapshot().await;
        let healthy_workers = workers
            .iter()
            .filter(|worker| worker.is_connected())
            .count();
        let degraded_workers = workers.iter().filter(|worker| worker.is_degraded()).count();
        let reconnecting_workers = workers
            .iter()
            .filter(|worker| worker.is_reconnecting())
            .count();
        let pool_degraded_reason =
            if healthy_workers == 0 && (degraded_workers > 0 || reconnecting_workers > 0) {
                Some("all_workers_unavailable")
            } else if degraded_workers > 0 || reconnecting_workers > 0 {
                Some("partial_pool_degraded")
            } else {
                None
            };
        let mut status = serde_json::Map::new();

        status.insert("connected".to_string(), (active_bridges > 0).into());
        status.insert("reconnect".to_string(), self.reconnect.into());
        status.insert(
            "shutting_down".to_string(),
            self.shutdown.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "transport_pool_size".to_string(),
            self.transport_pool_size.into(),
        );
        status.insert(
            "pool_policy".to_string(),
            json_value(self.pool_policy.clone()),
        );
        status.insert(
            "workload_hint".to_string(),
            json_value(self.workload_hint.clone()),
        );
        status.insert("active_bridges".to_string(), active_bridges.into());
        status.insert("healthy_workers".to_string(), healthy_workers.into());
        status.insert("degraded_workers".to_string(), degraded_workers.into());
        status.insert(
            "reconnecting_workers".to_string(),
            reconnecting_workers.into(),
        );
        status.insert(
            "pool_degraded_reason".to_string(),
            json_value(pool_degraded_reason),
        );
        status.insert(
            "selected_protocol".to_string(),
            json_value(selected_protocol.clone()),
        );
        status.insert(
            "tls_peer_auth_mode".to_string(),
            json_value(self.tls_peer_auth_mode.clone()),
        );
        status.insert("workers".to_string(), json_value(workers));
        status.insert(
            "uptime_secs".to_string(),
            self.started.elapsed().as_secs().into(),
        );
        status.insert(
            "generation".to_string(),
            self.generation.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "connect_attempts".to_string(),
            self.connect_attempts.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "successful_connects".to_string(),
            self.successful_connects.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "failed_connects".to_string(),
            self.failed_connects.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "active_tcp".to_string(),
            self.active_tcp.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "total_tcp".to_string(),
            self.total_tcp.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "tcp_open_attempts".to_string(),
            self.tcp_open_attempts.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "tcp_open_successes".to_string(),
            self.tcp_open_successes.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "tcp_open_failures".to_string(),
            self.tcp_open_failures.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "last_tcp_open_latency_ms".to_string(),
            json_value(last_sampled_u64(
                self.tcp_open_attempts.load(Ordering::Relaxed),
                self.last_tcp_open_latency_ms.load(Ordering::Relaxed),
            )),
        );
        status.insert(
            "ssh_direct_channel_open_samples".to_string(),
            self.ssh_direct_channel_open_samples
                .load(Ordering::Relaxed)
                .into(),
        );
        status.insert(
            "last_ssh_direct_channel_open_latency_ms".to_string(),
            json_value(last_sampled_u64(
                self.ssh_direct_channel_open_samples.load(Ordering::Relaxed),
                self.last_ssh_direct_channel_open_latency_ms
                    .load(Ordering::Relaxed),
            )),
        );
        status.insert(
            "spx_peer_handshake_samples".to_string(),
            self.spx_peer_handshake_samples
                .load(Ordering::Relaxed)
                .into(),
        );
        status.insert(
            "last_spx_peer_handshake_latency_ms".to_string(),
            json_value(last_sampled_u64(
                self.spx_peer_handshake_samples.load(Ordering::Relaxed),
                self.last_spx_peer_handshake_latency_ms
                    .load(Ordering::Relaxed),
            )),
        );
        status.insert(
            "bytes_client_to_remote".to_string(),
            self.bytes_client_to_remote.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "bytes_remote_to_client".to_string(),
            self.bytes_remote_to_client.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "spx_tcp_relay_samples".to_string(),
            self.spx_tcp_relay_samples.load(Ordering::Relaxed).into(),
        );
        status.insert(
            "last_spx_tcp_relay_duration_ms".to_string(),
            json_value(last_sampled_u64(
                self.spx_tcp_relay_samples.load(Ordering::Relaxed),
                self.last_spx_tcp_relay_duration_ms.load(Ordering::Relaxed),
            )),
        );
        status.insert(
            "last_spx_tcp_client_to_remote_bytes".to_string(),
            self.last_spx_tcp_client_to_remote_bytes
                .load(Ordering::Relaxed)
                .into(),
        );
        status.insert(
            "last_spx_tcp_remote_to_client_bytes".to_string(),
            self.last_spx_tcp_remote_to_client_bytes
                .load(Ordering::Relaxed)
                .into(),
        );
        status.insert(
            "last_spx_tcp_relay_close_reason".to_string(),
            json_value(self.last_spx_tcp_relay_close_reason.read().await.clone()),
        );
        status.insert(
            "link".to_string(),
            serde_json::json!({
                "health": {
                    "selected_protocol": selected_protocol,
                    "active_connections": active_bridges,
                    "active_streams": self.active_tcp.load(Ordering::Relaxed),
                    "active_channels": self.active_tcp.load(Ordering::Relaxed),
                    "pool_size": self.transport_pool_size,
                    "pool_policy": self.pool_policy.clone(),
                    "pool_workload_hint": self.workload_hint.clone(),
                    "open_attempts": self.tcp_open_attempts.load(Ordering::Relaxed),
                    "open_successes": self.tcp_open_successes.load(Ordering::Relaxed),
                    "open_failures": self.tcp_open_failures.load(Ordering::Relaxed),
                    "open_latency_ms": last_sampled_u64(
                        self.tcp_open_attempts.load(Ordering::Relaxed),
                        self.last_tcp_open_latency_ms.load(Ordering::Relaxed),
                    ),
                    "bytes_client_to_remote": self.bytes_client_to_remote.load(Ordering::Relaxed),
                    "bytes_remote_to_client": self.bytes_remote_to_client.load(Ordering::Relaxed),
                    "first_byte_latency_ms": serde_json::Value::Null,
                    "first_byte_samples": 0,
                    "max_first_byte_latency_ms": serde_json::Value::Null,
                    "last_close_reason": self.last_spx_tcp_relay_close_reason.read().await.clone(),
                    "degraded_reason": pool_degraded_reason,
                    "healthy_workers": healthy_workers,
                    "degraded_workers": degraded_workers,
                    "reconnecting_workers": reconnecting_workers,
                    "control_health": if active_bridges > 0 && pool_degraded_reason.is_none() {
                        "healthy"
                    } else if active_bridges > 0 {
                        "degraded"
                    } else if self.reconnect {
                        "reconnecting"
                    } else {
                        "disconnected"
                    },
                    "connected": active_bridges > 0,
                }
            }),
        );
        status.insert(
            "spx_frame_write_batches".to_string(),
            bridge_metrics.frame_write_batches.into(),
        );
        status.insert(
            "spx_frame_write_flushes".to_string(),
            bridge_metrics.frame_write_flushes.into(),
        );
        status.insert(
            "spx_frame_write_frames".to_string(),
            bridge_metrics.frame_write_frames.into(),
        );
        status.insert(
            "spx_frame_write_data_frames".to_string(),
            bridge_metrics.frame_write_data_frames.into(),
        );
        status.insert(
            "spx_frame_write_data_bytes".to_string(),
            bridge_metrics.frame_write_data_bytes.into(),
        );
        status.insert(
            "spx_frame_write_vectored_writes".to_string(),
            bridge_metrics.frame_write_vectored_writes.into(),
        );
        status.insert(
            "spx_frame_write_failures".to_string(),
            bridge_metrics.frame_write_failures.into(),
        );
        status.insert(
            "spx_frame_read_frames".to_string(),
            bridge_metrics.frame_read_frames.into(),
        );
        status.insert(
            "spx_frame_read_data_frames".to_string(),
            bridge_metrics.frame_read_data_frames.into(),
        );
        status.insert(
            "spx_frame_read_data_bytes".to_string(),
            bridge_metrics.frame_read_data_bytes.into(),
        );
        status.insert(
            "spx_tcp_stream_backpressure_timeouts".to_string(),
            bridge_metrics.tcp_stream_backpressure_timeouts.into(),
        );
        status.insert(
            "spx_udp_assoc_backpressure_timeouts".to_string(),
            bridge_metrics.udp_assoc_backpressure_timeouts.into(),
        );
        status.insert(
            "read_buffer_size".to_string(),
            protocol::TCP_DATA_CHUNK.into(),
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
            self.quic_options.receive_window.into(),
        );
        status.insert(
            "quic_stream_receive_window".to_string(),
            self.quic_options.stream_receive_window.into(),
        );
        status.insert(
            "quic_max_bidi_streams".to_string(),
            self.quic_options.max_bidi_streams.into(),
        );
        status.insert(
            "quic_keep_alive_interval_secs".to_string(),
            self.quic_options.keep_alive_interval_secs.into(),
        );
        status.insert(
            "quic_idle_timeout_secs".to_string(),
            self.quic_options.idle_timeout_secs.into(),
        );
        status.insert(
            "quic_runtime".to_string(),
            json_value(peer_transport::quic_runtime_diagnostics(self.quic_options)),
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
            "last_error".to_string(),
            json_value(self.last_error.read().await.clone()),
        );
        status.insert(
            "candidate_failures".to_string(),
            json_value(self.candidate_failures.read().await.clone()),
        );
        status.insert(
            "last_tcp_open_error".to_string(),
            json_value(self.last_tcp_open_error.read().await.clone()),
        );

        serde_json::Value::Object(status)
    }

    pub(super) async fn status_json(&self) -> Result<String> {
        let status = self.status_value().await;
        Ok(format!("{}\n", serde_json::to_string_pretty(&status)?))
    }
}

fn last_sampled_u64(sample_count: u64, value: u64) -> Option<u64> {
    (sample_count > 0).then_some(value)
}

fn json_value<T: serde::Serialize>(value: T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
}
