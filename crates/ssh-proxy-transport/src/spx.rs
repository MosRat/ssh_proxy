use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpxBridgeWorkerSnapshot {
    pub slot: usize,
    pub state: String,
    pub connected: bool,
    pub generation: u32,
    pub connect_attempts: u64,
    pub successful_connects: u64,
    pub failed_connects: u64,
    pub disconnects: u64,
    pub retry_count: u64,
    pub active_streams: u64,
    pub bytes_client_to_remote: u64,
    pub bytes_remote_to_client: u64,
    pub last_error: Option<String>,
    pub degraded_reason: Option<String>,
    pub selected_protocol: Option<String>,
    pub last_successful_protocol: Option<String>,
    pub last_event: Option<String>,
    pub last_connected_ago_secs: Option<u64>,
    pub last_disconnected_ago_secs: Option<u64>,
    pub last_failure_ago_secs: Option<u64>,
}

impl SpxBridgeWorkerSnapshot {
    pub fn is_connected(&self) -> bool {
        self.state == "connected"
    }

    pub fn is_degraded(&self) -> bool {
        self.state == "degraded"
    }

    pub fn is_reconnecting(&self) -> bool {
        self.state == "reconnecting"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_snapshot_classifies_public_states() {
        let snapshot = SpxBridgeWorkerSnapshot {
            slot: 0,
            state: "connected".to_string(),
            connected: true,
            generation: 1,
            connect_attempts: 1,
            successful_connects: 1,
            failed_connects: 0,
            disconnects: 0,
            retry_count: 0,
            active_streams: 2,
            bytes_client_to_remote: 10,
            bytes_remote_to_client: 20,
            last_error: None,
            degraded_reason: None,
            selected_protocol: Some("ssh-exec-spx".to_string()),
            last_successful_protocol: Some("ssh-exec-spx".to_string()),
            last_event: Some("bridge connected".to_string()),
            last_connected_ago_secs: Some(1),
            last_disconnected_ago_secs: None,
            last_failure_ago_secs: None,
        };

        assert!(snapshot.is_connected());
        assert!(!snapshot.is_degraded());
        assert!(!snapshot.is_reconnecting());
    }
}
