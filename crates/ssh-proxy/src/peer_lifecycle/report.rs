use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::repair;
pub(crate) use ssh_proxy_core::redaction::redact_value;

use super::{
    spec::PeerLifecycleSpec,
    workflow::{LifecycleOperation, PeerLifecyclePhase},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DependencyStatus {
    pub(crate) name: String,
    pub(crate) classification: String,
    pub(crate) state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) repair_action: Option<repair::RepairAction>,
}

impl DependencyStatus {
    pub(crate) fn new(
        name: impl Into<String>,
        classification: impl Into<String>,
        state: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            classification: classification.into(),
            state: state.into(),
            message: None,
            blocker: None,
            repair_action: None,
        }
    }

    pub(crate) fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub(crate) fn blocked(mut self, blocker: impl Into<String>) -> Self {
        let blocker = blocker.into();
        let mut object = Map::new();
        repair::attach_repair_action(&mut object, &blocker);
        self.repair_action = object
            .remove("repair_action")
            .and_then(|value| serde_json::from_value(value).ok());
        self.blocker = Some(blocker);
        self
    }

    pub(crate) fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({ "name": self.name }))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PeerLifecycleReport {
    pub(crate) target: String,
    pub(crate) state: String,
    pub(crate) phase: PeerLifecyclePhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) operation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) service_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) service_manager: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) artifacts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) health_probe: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) dependencies: Vec<DependencyStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) retry_after_ms: Option<u64>,
    pub(crate) recovery_attempts: u32,
    pub(crate) updated_at_unix: u64,
}

impl PeerLifecycleReport {
    pub(crate) fn new(target: impl Into<String>, phase: PeerLifecyclePhase) -> Self {
        Self {
            target: target.into(),
            state: phase.as_str().to_string(),
            phase,
            role: None,
            platform: None,
            scope: None,
            operation: None,
            provider: None,
            service_name: None,
            service_manager: None,
            artifacts: Vec::new(),
            health_probe: None,
            dependencies: Vec::new(),
            blocker: None,
            last_error: None,
            retry_after_ms: None,
            recovery_attempts: 0,
            updated_at_unix: now_unix(),
        }
    }

    pub(crate) fn apply_spec(&mut self, spec: &PeerLifecycleSpec, operation: LifecycleOperation) {
        self.role = Some(enum_json_name(&spec.role));
        self.platform = Some(enum_json_name(&spec.platform));
        self.scope = Some(enum_json_name(&spec.scope));
        self.operation = Some(operation.as_str().to_string());
        self.provider = Some(spec.provider.manager_name().to_string());
        self.service_manager = Some(spec.provider.manager_name().to_string());
        self.service_name = Some(spec.service_name.clone());
    }

    pub(crate) fn to_redacted_value(&self) -> Value {
        redact_value(&serde_json::to_value(self).unwrap_or_else(|_| Value::Null))
    }
}

fn enum_json_name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "unknown".to_string())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn report_redaction_hides_tokens_and_keeps_path_basenames() {
        let value = json!({
            "token": "secret",
            "identity": "C:/Users/me/.ssh/id_ed25519",
            "nested": {
                "known_hosts": ["C:/Users/me/.ssh/known_hosts"],
                "password": "also-secret",
                "safe": "ok"
            }
        });

        let redacted = redact_value(&value);

        assert_eq!(redacted["token"], "<redacted>");
        assert_eq!(redacted["identity"], "<redacted>/id_ed25519");
        assert_eq!(
            redacted["nested"]["known_hosts"][0],
            "<redacted>/known_hosts"
        );
        assert_eq!(redacted["nested"]["password"], "<redacted>");
        assert_eq!(redacted["nested"]["safe"], "ok");
    }

    #[test]
    fn dependency_status_attaches_repair_action() {
        let status = DependencyStatus::new("daemon_control", "required", "blocked")
            .blocked("daemon_unavailable");

        assert_eq!(status.blocker.as_deref(), Some("daemon_unavailable"));
        assert!(status.repair_action.is_some());
    }

    #[test]
    fn lifecycle_report_includes_symmetric_spec_fields() {
        let spec = crate::peer_lifecycle::spec::PeerLifecycleSpec::local_daemon(
            "local",
            "ssh_proxy",
            crate::peer_lifecycle::service_provider::ServiceProviderKind::SystemdUser,
            "ssh_proxy",
            None,
            None,
            None,
            "$HOME/.ssh_proxy",
        );
        let mut report = PeerLifecycleReport::new(
            "local",
            crate::peer_lifecycle::workflow::PeerLifecyclePhase::InstallService,
        );

        report.apply_spec(
            &spec,
            crate::peer_lifecycle::workflow::LifecycleOperation::Install,
        );
        let value = serde_json::to_value(report).unwrap();

        assert_eq!(value["role"], "local_daemon");
        assert_eq!(value["operation"], "install");
        assert_eq!(value["provider"], "systemd_user");
        assert_eq!(value["service_name"], "ssh_proxy");
    }
}
