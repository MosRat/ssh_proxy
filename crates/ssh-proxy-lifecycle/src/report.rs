use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    spec::PeerLifecycleSpec,
    workflow::{LifecycleOperation, PeerLifecyclePhase},
};
use ssh_proxy_core::{redaction::redact_value, repair};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyStatus {
    pub name: String,
    pub classification: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repair_action: Option<repair::RepairAction>,
}

impl DependencyStatus {
    pub fn new(
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

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn blocked(mut self, blocker: impl Into<String>) -> Self {
        let blocker = blocker.into();
        self.repair_action = repair::action_for_blocker(&blocker);
        self.blocker = Some(blocker);
        self
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({ "name": self.name }))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerLifecycleReport {
    pub target: String,
    pub state: String,
    pub phase: PeerLifecyclePhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_manager: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_probe: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<DependencyStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    pub recovery_attempts: u32,
    pub updated_at_unix: u64,
}

impl PeerLifecycleReport {
    pub fn new(target: impl Into<String>, phase: PeerLifecyclePhase) -> Self {
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

    pub fn apply_spec(&mut self, spec: &PeerLifecycleSpec, operation: LifecycleOperation) {
        self.role = Some(enum_json_name(&spec.role));
        self.platform = Some(enum_json_name(&spec.platform));
        self.scope = Some(enum_json_name(&spec.scope));
        self.operation = Some(operation.as_str().to_string());
        self.provider = Some(spec.provider.manager_name().to_string());
        self.service_manager = Some(spec.provider.manager_name().to_string());
        self.service_name = Some(spec.service_name.clone());
    }

    pub fn to_redacted_value(&self) -> Value {
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
    use serde_json::json;
    use ssh_proxy_core::{intent::SshTargetIntent, model::PersistenceMode};

    use super::*;
    use crate::{service_provider::ServiceProviderKind, spec::PeerLifecycleSpec};

    #[test]
    fn lifecycle_report_uses_shared_spec_fields() {
        let intent = ssh_proxy_core::intent::RemoteInstallIntent::new(
            SshTargetIntent::new("edge"),
            "127.0.0.1:19080".parse().unwrap(),
            "127.0.0.1:19081".parse().unwrap(),
            PersistenceMode::Systemd,
        );
        let spec = PeerLifecycleSpec::remote_peer_from_intent(
            "edge",
            "/tmp/ssh_proxy",
            &intent,
            ServiceProviderKind::SystemdUser,
        );
        let mut report = PeerLifecycleReport::new("edge", PeerLifecyclePhase::InstallService);
        report.apply_spec(&spec, LifecycleOperation::Install);

        assert_eq!(report.role.as_deref(), Some("remote_peer"));
        assert_eq!(report.provider.as_deref(), Some("systemd_user"));
        assert_eq!(report.operation.as_deref(), Some("install"));
    }

    #[test]
    fn report_redaction_hides_tokens_and_keeps_path_basenames() {
        let value = json!({
            "token": "secret",
            "identity": "C:/Users/me/.ssh/id_ed25519",
            "known_hosts": "/home/me/.ssh/known_hosts",
            "safe": "value"
        });

        let redacted = redact_value(&value);

        assert_eq!(redacted["token"], "<redacted>");
        assert_eq!(redacted["identity"], "<redacted>/id_ed25519");
        assert_eq!(redacted["known_hosts"], "<redacted>/known_hosts");
        assert_eq!(redacted["safe"], "value");
    }
}
