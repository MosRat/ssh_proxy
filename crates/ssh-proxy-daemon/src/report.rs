use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonEnvelopeReport {
    pub ok: bool,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon_api: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

impl DaemonEnvelopeReport {
    pub fn new(kind: impl Into<String>, ok: bool) -> Self {
        Self {
            ok,
            kind: kind.into(),
            daemon_api: Some("v0.3".to_string()),
            job: None,
            session: None,
            peer: None,
            route: None,
            next_action: None,
            retry_after_ms: None,
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| {
            json!({
                "ok": false,
                "kind": "daemon_report",
                "daemon_api": "v0.3",
                "error": "failed to encode daemon report",
            })
        })
    }
}
