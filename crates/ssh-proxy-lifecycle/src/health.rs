use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

pub type PeerHealthState = ssh_proxy_core::report::HealthStatus;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerHealth {
    pub state: PeerHealthState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<SocketAddr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
}

impl PeerHealth {
    pub fn healthy(transport: SocketAddr, control: impl Into<String>) -> Self {
        Self {
            state: PeerHealthState::Healthy,
            transport: Some(transport),
            control: Some(control.into()),
            blocker: None,
            last_error: None,
            latency_ms: None,
        }
    }

    pub fn failed(blocker: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            state: PeerHealthState::Failed,
            transport: None,
            control: None,
            blocker: Some(blocker.into()),
            last_error: Some(error.into()),
            latency_ms: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_health_serializes_stable_state_names() {
        let health =
            PeerHealth::healthy("127.0.0.1:19080".parse().unwrap(), "tcp://127.0.0.1:19081");
        let value = serde_json::to_value(health).unwrap();

        assert_eq!(value["state"], "healthy");
        assert_eq!(value["transport"], "127.0.0.1:19080");
    }
}
