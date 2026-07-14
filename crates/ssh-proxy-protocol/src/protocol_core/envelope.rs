use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::version::ControlApiVersion;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlEnvelope<T> {
    pub api_version: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub payload: T,
}

impl<T> ControlEnvelope<T> {
    pub fn new(payload: T) -> Self {
        Self {
            api_version: ControlApiVersion::current().value(),
            kind: None,
            payload,
        }
    }

    pub fn with_kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = Some(kind.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repair_action: Option<Value>,
}

impl ControlError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            blocker: None,
            repair_action: None,
        }
    }

    pub fn with_blocker(mut self, blocker: impl Into<String>) -> Self {
        self.blocker = Some(blocker.into());
        self
    }

    pub fn with_repair_action(mut self, repair_action: Value) -> Self {
        if !repair_action.is_null() {
            self.repair_action = Some(repair_action);
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlResponse<T> {
    pub api_version: u16,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repair_action: Option<Value>,
}

impl<T> ControlResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            api_version: ControlApiVersion::current().value(),
            ok: true,
            kind: None,
            code: None,
            message: None,
            error: None,
            data: Some(data),
            blocker: None,
            repair_action: None,
        }
    }

    pub fn ok_message(message: impl Into<String>) -> Self {
        Self {
            api_version: ControlApiVersion::current().value(),
            ok: true,
            kind: None,
            code: None,
            message: Some(message.into()),
            error: None,
            data: None,
            blocker: None,
            repair_action: None,
        }
    }

    pub fn error(error: ControlError) -> Self {
        Self {
            api_version: ControlApiVersion::current().value(),
            ok: false,
            kind: None,
            code: Some(error.code),
            message: None,
            error: Some(error.message),
            data: None,
            blocker: error.blocker,
            repair_action: error.repair_action,
        }
    }

    pub fn with_kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = Some(kind.into());
        self
    }
}

impl ControlResponse<Value> {
    pub fn public_ok_value(value: Value) -> Value {
        match value {
            Value::Object(mut object) => {
                object
                    .entry("api_version")
                    .or_insert_with(|| json!(ControlApiVersion::current().value()));
                Value::Object(object)
            }
            other => serde_json::to_value(Self::ok(other)).unwrap_or_else(|_| {
                json!({
                    "api_version": ControlApiVersion::current().value(),
                    "ok": false,
                    "code": "internal",
                    "error": "failed to encode control response"
                })
            }),
        }
    }

    pub fn error_value(code: impl Into<String>, error: impl Into<String>) -> Value {
        serde_json::to_value(Self::error(ControlError::new(code, error))).unwrap_or_else(|_| {
            json!({
                "api_version": ControlApiVersion::current().value(),
                "ok": false,
                "code": "internal",
                "error": "failed to encode control response"
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_response_keeps_public_fields() {
        let value = serde_json::to_value(ControlResponse::ok(json!({"answer": 42}))).unwrap();

        assert_eq!(value["api_version"], 1);
        assert_eq!(value["ok"], true);
        assert_eq!(value["data"]["answer"], 42);
        assert!(value.get("error").is_none());
    }

    #[test]
    fn message_response_matches_daemon_shape() {
        let value = serde_json::to_value(ControlResponse::<Value>::ok_message("ready")).unwrap();

        assert_eq!(value["api_version"], 1);
        assert_eq!(value["ok"], true);
        assert_eq!(value["message"], "ready");
        assert!(value.get("data").is_none());
    }

    #[test]
    fn error_response_keeps_blocker_and_repair_action() {
        let response = ControlResponse::<Value>::error(
            ControlError::new("remote_port_refused", "remote port is not ready")
                .with_blocker("remote_port_refused")
                .with_repair_action(json!({"kind": "proxy_session_retry"})),
        );
        let value = serde_json::to_value(response).unwrap();

        assert_eq!(value["api_version"], 1);
        assert_eq!(value["ok"], false);
        assert_eq!(value["code"], "remote_port_refused");
        assert_eq!(value["error"], "remote port is not ready");
        assert_eq!(value["blocker"], "remote_port_refused");
        assert_eq!(value["repair_action"]["kind"], "proxy_session_retry");
    }

    #[test]
    fn public_ok_value_preserves_existing_object_shape() {
        let value = ControlResponse::public_ok_value(json!({
            "ok": true,
            "kind": "status",
            "answer": 42
        }));

        assert_eq!(value["api_version"], 1);
        assert_eq!(value["kind"], "status");
        assert_eq!(value["answer"], 42);
        assert!(value.get("data").is_none());
    }

    #[test]
    fn public_ok_value_wraps_non_object_payload() {
        let value = ControlResponse::public_ok_value(json!(["route-1"]));

        assert_eq!(value["api_version"], 1);
        assert_eq!(value["ok"], true);
        assert_eq!(value["data"][0], "route-1");
    }
}
