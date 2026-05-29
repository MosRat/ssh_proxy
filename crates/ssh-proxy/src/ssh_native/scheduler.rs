pub(super) const RECENT_FAILURE_WINDOW_MS: u64 = 30_000;
pub(super) const SSH_SESSION_GROWTH_MIN_ACTIVE_CHANNELS: u32 = 2;

#[derive(Debug, Clone, Copy)]
pub(super) struct SessionScoreComponents {
    pub(super) active_channels: u64,
    pub(super) open_failures: u64,
    pub(super) last_open_latency_ms: u64,
    pub(super) first_byte_latency_ms: u64,
    pub(super) bytes_in_flight: u64,
    pub(super) recent_failure_penalty: u64,
    pub(super) error_closes: u64,
}

impl SessionScoreComponents {
    pub(super) fn score(self) -> u64 {
        calculate_session_score(self)
    }

    pub(super) fn to_json(self) -> serde_json::Value {
        serde_json::json!({
            "active_channels": self.active_channels,
            "open_failures": self.open_failures,
            "last_open_latency_ms": self.last_open_latency_ms,
            "first_byte_latency_ms": self.first_byte_latency_ms,
            "bytes_in_flight": self.bytes_in_flight,
            "recent_failure_penalty": self.recent_failure_penalty,
            "error_closes": self.error_closes,
        })
    }
}

pub(super) fn should_open_new_session(
    existing_sessions: usize,
    pool_size: usize,
    min_active_channels: u32,
) -> bool {
    existing_sessions == 0
        || (existing_sessions < pool_size
            && min_active_channels >= SSH_SESSION_GROWTH_MIN_ACTIVE_CHANNELS)
}

pub(super) fn calculate_session_score(components: SessionScoreComponents) -> u64 {
    components
        .active_channels
        .saturating_mul(10_000)
        .saturating_add(components.open_failures.saturating_mul(1_000))
        .saturating_add(components.error_closes.saturating_mul(750))
        .saturating_add(components.last_open_latency_ms.min(5_000))
        .saturating_add(components.first_byte_latency_ms.min(5_000))
        .saturating_add(components.bytes_in_flight / (1024 * 1024))
        .saturating_add(components.recent_failure_penalty)
}
