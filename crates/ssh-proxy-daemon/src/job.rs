use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ssh_proxy_core::repair::{RepairAction, action_for_blocker};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonJobState {
    Queued,
    Running,
    WaitingRetry,
    Healthy,
    Failed,
    Cancelled,
}

impl DaemonJobState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Healthy | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonJobPhase {
    Queued,
    Reconciling,
    ResolveTarget,
    ValidateLocalProxy,
    SelectRemotePort,
    EnsureLocalProxy,
    EnsurePeer,
    InspectPeerDescriptor,
    DependencyCheck,
    StageRemotePeer,
    WritePeerConfig,
    InstallPeerService,
    StartPeerService,
    PeerHealthProbe,
    RecordPeer,
    EnsureTransport,
    PlanRoute,
    StartRoute,
    WaitRouteReady,
    VerifyRemotePort,
    ApplyRemoteSettings,
    HealthMonitoring,
    StageUpdate,
    VerifyUpdate,
    SwitchBinary,
    RestartDaemon,
    Rollback,
    Healthy,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonJobRecord {
    pub id: String,
    pub kind: String,
    pub state: DaemonJobState,
    pub phase: DaemonJobPhase,
    pub progress: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repair_action: Option<RepairAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub recovery_attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

impl DaemonJobRecord {
    pub fn new(id: impl Into<String>, kind: impl Into<String>) -> Self {
        let now = now_unix();
        Self {
            id: id.into(),
            kind: kind.into(),
            state: DaemonJobState::Queued,
            phase: DaemonJobPhase::Queued,
            progress: 0,
            blocker: None,
            next_action: None,
            repair_action: None,
            last_error: None,
            retry_after_ms: None,
            recovery_attempts: 0,
            target: None,
            workspace_id: None,
            route_id: None,
            remote_url: None,
            created_at_unix: now,
            updated_at_unix: now,
        }
    }

    pub fn transition(
        mut self,
        state: DaemonJobState,
        phase: DaemonJobPhase,
        progress: u8,
    ) -> Self {
        self.state = state;
        self.phase = phase;
        self.progress = progress.min(100);
        self.updated_at_unix = now_unix();
        if !matches!(state, DaemonJobState::Failed) {
            self.last_error = None;
        }
        if !matches!(state, DaemonJobState::WaitingRetry) {
            self.retry_after_ms = None;
        }
        self
    }

    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn with_workspace(mut self, workspace_id: Option<String>) -> Self {
        self.workspace_id = workspace_id;
        self
    }

    pub fn with_route(mut self, route_id: impl Into<String>) -> Self {
        self.route_id = Some(route_id.into());
        self
    }

    pub fn with_remote_url(mut self, remote_url: Option<String>) -> Self {
        self.remote_url = remote_url;
        self
    }

    pub fn with_next_action(mut self, next_action: impl Into<String>) -> Self {
        self.next_action = Some(next_action.into());
        self
    }

    pub fn with_retry_after_ms(mut self, retry_after_ms: u64) -> Self {
        self.retry_after_ms = Some(retry_after_ms);
        self
    }

    pub fn with_recovery_attempts(mut self, recovery_attempts: u32) -> Self {
        self.recovery_attempts = recovery_attempts;
        self
    }

    pub fn failed(mut self, error: impl Into<String>, blocker: Option<String>) -> Self {
        self.state = DaemonJobState::Failed;
        self.phase = DaemonJobPhase::Failed;
        self.progress = 100;
        self.last_error = Some(error.into());
        self.repair_action = blocker.as_deref().and_then(action_for_blocker);
        self.blocker = blocker;
        self.retry_after_ms = None;
        self.updated_at_unix = now_unix();
        self
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| {
            json!({
                "id": self.id,
                "kind": self.kind,
                "state": "failed",
                "phase": "failed",
                "last_error": "failed to encode job record",
            })
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonJobEvent {
    pub job_id: String,
    pub state: DaemonJobState,
    pub phase: DaemonJobPhase,
    pub message: String,
    pub created_at_unix: u64,
}

pub fn job_health(job: &DaemonJobRecord) -> &'static str {
    match job.state {
        DaemonJobState::Healthy => "healthy",
        DaemonJobState::Failed => "failed",
        DaemonJobState::Cancelled => "cancelled",
        DaemonJobState::Queued | DaemonJobState::Running | DaemonJobState::WaitingRetry => {
            "starting"
        }
    }
}

pub fn enum_value<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn is_zero(value: &u32) -> bool {
    *value == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_record_preserves_legacy_json_shape() {
        let record = DaemonJobRecord::new("session:abc", "ensure_proxy_session")
            .with_target("remote")
            .transition(
                DaemonJobState::WaitingRetry,
                DaemonJobPhase::VerifyRemotePort,
                85,
            )
            .with_retry_after_ms(250);
        let value = record.to_value();

        assert_eq!(value["id"], "session:abc");
        assert_eq!(value["kind"], "ensure_proxy_session");
        assert_eq!(value["state"], "waiting_retry");
        assert_eq!(value["phase"], "verify_remote_port");
        assert_eq!(value["progress"], 85);
        assert_eq!(value["retry_after_ms"], 250);
    }

    #[test]
    fn failed_job_attaches_repair_action() {
        let record = DaemonJobRecord::new("remote-peer:host", "ensure_remote_peer").failed(
            "install failed",
            Some("remote_peer_install_failed".to_string()),
        );
        let value = record.to_value();

        assert_eq!(value["state"], "failed");
        assert_eq!(value["phase"], "failed");
        assert_eq!(value["blocker"], "remote_peer_install_failed");
        assert_eq!(value["repair_action"]["kind"], "remote_peer_repair");
    }
}
