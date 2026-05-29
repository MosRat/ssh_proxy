use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::job::now_unix;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStateRecord {
    pub schema_version: u32,
    pub version: String,
    pub health: String,
    pub update_state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub started_at_unix: u64,
    pub updated_at_unix: u64,
}

impl DaemonStateRecord {
    pub fn new(version: impl Into<String>) -> Self {
        let now = now_unix();
        Self {
            schema_version: 1,
            version: version.into(),
            health: "starting".to_string(),
            update_state: DaemonUpdateState::Idle.as_str().to_string(),
            update: None,
            control_endpoint: None,
            name: None,
            started_at_unix: now,
            updated_at_unix: now,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonUpdateState {
    Idle,
    Requested,
    Staged,
    Switching,
    Healthy,
    Blocked,
    Failed,
}

impl DaemonUpdateState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Requested => "requested",
            Self::Staged => "staged",
            Self::Switching => "switching",
            Self::Healthy => "healthy",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_state_record_preserves_public_fields() {
        let value = serde_json::to_value(DaemonStateRecord::new("0.1.1")).unwrap();

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["version"], "0.1.1");
        assert_eq!(value["health"], "starting");
        assert_eq!(value["update_state"], "idle");
    }
}
