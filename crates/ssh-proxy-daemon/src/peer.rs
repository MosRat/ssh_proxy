use serde::{Deserialize, Serialize};
use serde_json::Value;
use ssh_proxy_core::repair::RepairAction;

use crate::job::{is_zero, now_unix};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerStatusRecord {
    pub target: String,
    pub state: String,
    #[serde(default = "default_unknown_health")]
    pub health: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transport_protocols: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_manager: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descriptor_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency_report: Option<Value>,
    pub update_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repair_action: Option<RepairAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub recovery_attempts: u32,
    pub updated_at_unix: u64,
}

impl PeerStatusRecord {
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            state: "unknown".to_string(),
            health: default_unknown_health(),
            version: None,
            control_endpoint: None,
            transport: None,
            transport_protocols: Vec::new(),
            service_manager: None,
            descriptor_hash: None,
            install: None,
            dependency_report: None,
            update_required: false,
            blocker: None,
            repair_action: None,
            last_error: None,
            retry_after_ms: None,
            recovery_attempts: 0,
            updated_at_unix: now_unix(),
        }
    }
}

fn default_unknown_health() -> String {
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_status_record_omits_empty_optional_fields() {
        let value = serde_json::to_value(PeerStatusRecord::new("remote")).unwrap();

        assert_eq!(value["target"], "remote");
        assert_eq!(value["health"], "unknown");
        assert!(value.get("control_endpoint").is_none());
        assert!(value.get("transport_protocols").is_none());
    }
}
