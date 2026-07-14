use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ssh_proxy_core::repair::RepairAction;

use crate::job::{DaemonJobRecord, enum_value, is_zero, job_health, now_unix};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxySessionRecord {
    pub session_id: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_paths: Vec<String>,
    pub job_id: String,
    pub route_id: String,
    pub local_proxy: String,
    pub remote_bind: String,
    pub remote_port: u16,
    pub remote_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply_policy: Option<Value>,
    pub state: String,
    pub phase: String,
    pub health: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repair_action: Option<RepairAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub remote_setup: RemoteSetupStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_probe: Option<Value>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub recovery_attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<Value>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

impl ProxySessionRecord {
    pub fn from_job(
        session_id: impl Into<String>,
        target: impl Into<String>,
        route_id: impl Into<String>,
        local_proxy: impl Into<String>,
        remote_bind: impl Into<String>,
        remote_port: u16,
        remote_url: impl Into<String>,
        job: &DaemonJobRecord,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            target: target.into(),
            workspace_id: job.workspace_id.clone(),
            ssh: None,
            workspace_paths: Vec::new(),
            job_id: job.id.clone(),
            route_id: route_id.into(),
            local_proxy: local_proxy.into(),
            remote_bind: remote_bind.into(),
            remote_port,
            remote_url: job.remote_url.clone().unwrap_or_else(|| remote_url.into()),
            apply_policy: None,
            state: enum_value(&job.state),
            phase: enum_value(&job.phase),
            health: job_health(job).to_string(),
            blocker: job.blocker.clone(),
            next_action: job.next_action.clone(),
            repair_action: job.repair_action.clone(),
            last_error: job.last_error.clone(),
            remote_setup: RemoteSetupStatus::pending(),
            handoff_probe: None,
            recovery_attempts: job.recovery_attempts,
            route: None,
            created_at_unix: job.created_at_unix,
            updated_at_unix: job.updated_at_unix,
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({ "session_id": self.session_id }))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSetupStatus {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desired_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_hash: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub drift_detected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub updated_at_unix: u64,
}

impl RemoteSetupStatus {
    pub fn pending() -> Self {
        Self {
            state: "pending".to_string(),
            last_hash: None,
            desired_hash: None,
            applied_hash: None,
            drift_detected: false,
            remote_url: None,
            verified: false,
            next_action: Some("wait_for_proxy_session_job".to_string()),
            last_error: None,
            updated_at_unix: now_unix(),
        }
    }

    pub fn required() -> Self {
        Self {
            state: "required".to_string(),
            next_action: Some("apply_remote_settings".to_string()),
            ..Self::pending()
        }
    }

    pub fn running(desired_hash: Option<String>, remote_url: Option<String>) -> Self {
        Self {
            state: "running".to_string(),
            desired_hash,
            remote_url,
            next_action: Some("wait_for_remote_setup".to_string()),
            updated_at_unix: now_unix(),
            ..Self::pending()
        }
    }

    pub fn applied(
        desired_hash: String,
        applied_hash: String,
        remote_url: String,
        verified: bool,
    ) -> Self {
        Self {
            state: "applied".to_string(),
            last_hash: Some(applied_hash.clone()),
            desired_hash: Some(desired_hash),
            applied_hash: Some(applied_hash),
            drift_detected: false,
            remote_url: Some(remote_url),
            verified,
            next_action: Some("monitor_remote_setup_drift".to_string()),
            last_error: None,
            updated_at_unix: now_unix(),
        }
    }

    pub fn failed(error: String, desired_hash: Option<String>, remote_url: Option<String>) -> Self {
        Self {
            state: "failed".to_string(),
            desired_hash,
            remote_url,
            next_action: Some("rerun_apply_remote_settings".to_string()),
            last_error: Some(error),
            updated_at_unix: now_unix(),
            ..Self::pending()
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use crate::job::{DaemonJobPhase, DaemonJobState};

    use super::*;

    #[test]
    fn proxy_session_record_uses_job_state_shape() {
        let job = DaemonJobRecord::new("session:window-a", "ensure_proxy_session")
            .with_workspace(Some("Window A".to_string()))
            .with_remote_url(Some("http://127.0.0.1:17890".to_string()))
            .transition(DaemonJobState::Running, DaemonJobPhase::EnsurePeer, 35);
        let record = ProxySessionRecord::from_job(
            "session:window-a",
            "remote",
            "route:window-a",
            "http://127.0.0.1:10808/",
            "127.0.0.1",
            17890,
            "http://127.0.0.1:17890",
            &job,
        );
        let value = record.to_value();

        assert_eq!(value["session_id"], "session:window-a");
        assert_eq!(value["state"], "running");
        assert_eq!(value["phase"], "ensure_peer");
        assert_eq!(value["health"], "starting");
    }

    #[test]
    fn remote_setup_status_keeps_legacy_actions() {
        let value = serde_json::to_value(RemoteSetupStatus::required()).unwrap();

        assert_eq!(value["state"], "required");
        assert_eq!(value["next_action"], "apply_remote_settings");
    }
}
