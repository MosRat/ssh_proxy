use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

use crate::protocol_core::envelope::{ControlError, ControlResponse};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NodeResponse {
    pub(crate) api_version: u16,
    pub(crate) ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) repair_action: Option<Value>,
}

impl NodeResponse {
    pub(crate) fn ok_message(message: impl Into<String>) -> Self {
        Self::from_control(ControlResponse::<Value>::ok_message(message))
    }

    pub(crate) fn error(code: impl Into<String>, error: impl Into<String>) -> Self {
        Self::from_control(ControlResponse::error(ControlError::new(code, error)))
    }

    pub(crate) fn to_line(&self) -> Result<String> {
        Ok(format!("{}\n", serde_json::to_string_pretty(self)?))
    }

    pub(crate) fn error_line(code: impl Into<String>, error: impl Into<String>) -> String {
        Self::error(code, error)
            .to_line()
            .unwrap_or_else(|_| "{\"api_version\":1,\"ok\":false,\"code\":\"internal\",\"error\":\"failed to encode node response\"}\n".to_string())
    }

    fn from_control(response: ControlResponse<Value>) -> Self {
        Self {
            api_version: response.api_version,
            ok: response.ok,
            code: response.code,
            message: response.message,
            error: response.error,
            data: response.data,
            blocker: response.blocker,
            repair_action: response.repair_action,
        }
    }
}

pub(crate) fn response_line(value: Value) -> Result<String> {
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&response_value(value))?
    ))
}

pub(crate) fn response_value(value: Value) -> Value {
    ControlResponse::public_ok_value(value)
}
