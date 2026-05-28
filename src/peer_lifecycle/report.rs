use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::repair;

use super::workflow::PeerLifecyclePhase;

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
    pub(crate) service_manager: Option<String>,
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
            service_manager: None,
            dependencies: Vec::new(),
            blocker: None,
            last_error: None,
            retry_after_ms: None,
            recovery_attempts: 0,
            updated_at_unix: now_unix(),
        }
    }

    pub(crate) fn to_redacted_value(&self) -> Value {
        redact_value(&serde_json::to_value(self).unwrap_or_else(|_| Value::Null))
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(crate) fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(redact_object(object)),
        Value::Array(array) => Value::Array(array.iter().map(redact_value).collect()),
        other => other.clone(),
    }
}

fn redact_object(object: &Map<String, Value>) -> Map<String, Value> {
    let mut redacted = Map::new();
    for (key, value) in object {
        let lower = key.to_ascii_lowercase();
        if lower.contains("token")
            || lower.contains("password")
            || lower.contains("passphrase")
            || lower.contains("secret")
            || lower.contains("credential")
        {
            redacted.insert(key.clone(), json!("<redacted>"));
            continue;
        }
        if lower.contains("identity") || lower.contains("known_hosts") {
            redacted.insert(key.clone(), redact_pathish(value));
            continue;
        }
        redacted.insert(key.clone(), redact_value(value));
    }
    redacted
}

fn redact_pathish(value: &Value) -> Value {
    match value {
        Value::String(path) => json!(redacted_path(path)),
        Value::Array(values) => Value::Array(values.iter().map(redact_pathish).collect()),
        _ => redact_value(value),
    }
}

fn redacted_path(path: &str) -> String {
    let file = std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<path>");
    format!("<redacted>/{file}")
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
