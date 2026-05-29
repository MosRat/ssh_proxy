use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::repair;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HealthStatus {
    Unknown,
    Starting,
    WaitingRetry,
    Healthy,
    Degraded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DependencyClassification {
    Required,
    Optional,
    DiagnosticOnly,
    EmergencyCompat,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RepairActionRef {
    pub(crate) id: String,
    pub(crate) kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) command: Option<String>,
    pub(crate) interactive: bool,
    pub(crate) requires_elevation: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) retry_after_ms: Option<u64>,
}

impl From<&repair::RepairAction> for RepairActionRef {
    fn from(action: &repair::RepairAction) -> Self {
        Self {
            id: action.id.clone(),
            kind: action.kind.clone(),
            command: action.command.clone(),
            interactive: action.interactive,
            requires_elevation: action.requires_elevation,
            retry_after_ms: action.retry_after_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct DependencyStatusReport {
    pub(crate) name: String,
    pub(crate) classification: DependencyClassification,
    pub(crate) state: HealthStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) repair_action: Option<RepairActionRef>,
}

impl DependencyStatusReport {
    pub(crate) fn required(name: impl Into<String>, state: HealthStatus) -> Self {
        Self {
            name: name.into(),
            classification: DependencyClassification::Required,
            state,
            message: None,
            blocker: None,
            repair_action: None,
        }
    }

    pub(crate) fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub(crate) fn with_blocker(mut self, blocker: impl Into<String>) -> Self {
        let blocker = blocker.into();
        self.repair_action = repair::action_for_blocker(&blocker)
            .as_ref()
            .map(Into::into);
        self.blocker = Some(blocker);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RuntimeDecisionReport {
    pub(crate) selected_transport: String,
    pub(crate) source: String,
    pub(crate) reason: String,
    #[serde(default)]
    pub(crate) requires_external_ssh: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) details: Option<Value>,
}

impl RuntimeDecisionReport {
    pub(crate) fn new(
        selected_transport: impl Into<String>,
        source: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            selected_transport: selected_transport.into(),
            source: source.into(),
            reason: reason.into(),
            requires_external_ssh: false,
            endpoint: None,
            details: None,
        }
    }

    pub(crate) fn requires_external_ssh(mut self, value: bool) -> Self {
        self.requires_external_ssh = value;
        self
    }

    pub(crate) fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    pub(crate) fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_status_uses_stable_snake_case_names() {
        let value = serde_json::to_value(HealthStatus::WaitingRetry).unwrap();

        assert_eq!(value, "waiting_retry");
    }

    #[test]
    fn dependency_status_report_attaches_repair_reference() {
        let report = DependencyStatusReport::required("daemon", HealthStatus::Failed)
            .with_blocker("daemon_unavailable");
        let value = serde_json::to_value(report).unwrap();

        assert_eq!(value["classification"], "required");
        assert_eq!(value["state"], "failed");
        assert_eq!(value["blocker"], "daemon_unavailable");
        assert_eq!(value["repair_action"]["kind"], "daemon_install");
    }

    #[test]
    fn runtime_decision_report_has_shared_transport_shape() {
        let report = RuntimeDecisionReport::new("ssh-exec", "cli", "explicit compatibility")
            .requires_external_ssh(true)
            .with_endpoint("tcp://127.0.0.1:19080");
        let value = serde_json::to_value(report).unwrap();

        assert_eq!(value["selected_transport"], "ssh-exec");
        assert_eq!(value["source"], "cli");
        assert_eq!(value["requires_external_ssh"], true);
        assert_eq!(value["endpoint"], "tcp://127.0.0.1:19080");
    }
}
