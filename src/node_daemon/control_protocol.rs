use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::cli;

pub(crate) const NODE_CONTROL_VERSION: u16 = 1;

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
}

impl NodeResponse {
    pub(crate) fn ok_message(message: impl Into<String>) -> Self {
        Self {
            api_version: NODE_CONTROL_VERSION,
            ok: true,
            code: None,
            message: Some(message.into()),
            error: None,
            data: None,
        }
    }

    pub(crate) fn error(code: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            api_version: NODE_CONTROL_VERSION,
            ok: false,
            code: Some(code.into()),
            message: None,
            error: Some(error.into()),
            data: None,
        }
    }

    pub(crate) fn to_line(&self) -> Result<String> {
        Ok(format!("{}\n", serde_json::to_string_pretty(self)?))
    }

    pub(crate) fn error_line(code: impl Into<String>, error: impl Into<String>) -> String {
        Self::error(code, error)
            .to_line()
            .unwrap_or_else(|_| "{\"api_version\":1,\"ok\":false,\"code\":\"internal\",\"error\":\"failed to encode node response\"}\n".to_string())
    }
}

pub(crate) fn response_line(value: Value) -> Result<String> {
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&response_value(value))?
    ))
}

pub(crate) fn response_value(value: Value) -> Value {
    match value {
        Value::Object(mut object) => {
            object
                .entry("api_version")
                .or_insert_with(|| json!(NODE_CONTROL_VERSION));
            Value::Object(object)
        }
        other => {
            let mut object = Map::new();
            object.insert("api_version".to_string(), json!(NODE_CONTROL_VERSION));
            object.insert("ok".to_string(), json!(true));
            object.insert("data".to_string(), other);
            Value::Object(object)
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct NodeRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) api_version: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) auth_token: Option<String>,
    pub(crate) cmd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) direction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) persist: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) proxy: Option<cli::ProxyArgs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reverse: Option<cli::ReverseTaskArgs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) route: Option<cli::RouteArgs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) bootstrap: Option<cli::PeerBootstrapArgs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) connect_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fallback_reason: Option<String>,
}

impl NodeRequest {
    pub(crate) fn command(cmd: impl Into<String>) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: cmd.into(),
            ..Self::default()
        }
    }

    pub(crate) fn legacy_command(cmd: String, profile: Option<String>) -> Self {
        Self {
            cmd,
            profile,
            ..Self::default()
        }
    }

    pub(crate) fn connect(profile: String) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "connect".to_string(),
            profile: Some(profile),
            ..Self::default()
        }
    }

    pub(crate) fn disconnect(profile: String) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "disconnect".to_string(),
            profile: Some(profile),
            ..Self::default()
        }
    }

    pub(crate) fn route_start_forward(
        id: impl Into<String>,
        persist: bool,
        proxy: cli::ProxyArgs,
    ) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "route_start".to_string(),
            id: Some(id.into()),
            direction: Some("forward".to_string()),
            persist: Some(persist),
            proxy: Some(proxy),
            ..Self::default()
        }
    }

    pub(crate) fn route_start_reverse(
        id: impl Into<String>,
        persist: bool,
        reverse: cli::ReverseTaskArgs,
        connect_mode: Option<String>,
    ) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "route_start".to_string(),
            id: Some(id.into()),
            direction: Some("reverse".to_string()),
            persist: Some(persist),
            reverse: Some(reverse),
            connect_mode,
            ..Self::default()
        }
    }

    pub(crate) fn route_stop(id: String) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "route_stop".to_string(),
            id: Some(id),
            ..Self::default()
        }
    }

    pub(crate) fn route_restart(id: String) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "route_restart".to_string(),
            id: Some(id),
            ..Self::default()
        }
    }

    pub(crate) fn route_intent(route: cli::RouteArgs) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "route_intent".to_string(),
            route: Some(route),
            ..Self::default()
        }
    }

    pub(crate) fn route_plan(route: cli::RouteArgs) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "route_plan".to_string(),
            route: Some(route),
            ..Self::default()
        }
    }

    pub(crate) fn peer_bootstrap(bootstrap: cli::PeerBootstrapArgs) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "peer_bootstrap".to_string(),
            bootstrap: Some(bootstrap),
            ..Self::default()
        }
    }

    pub(crate) fn peer_refresh(bootstrap: cli::PeerBootstrapArgs) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "peer_refresh".to_string(),
            bootstrap: Some(bootstrap),
            ..Self::default()
        }
    }

    pub(crate) fn peer_diff(bootstrap: cli::PeerBootstrapArgs) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "peer_diff".to_string(),
            bootstrap: Some(bootstrap),
            ..Self::default()
        }
    }

    pub(crate) fn peer_reconcile(bootstrap: cli::PeerBootstrapArgs) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "peer_reconcile".to_string(),
            bootstrap: Some(bootstrap),
            ..Self::default()
        }
    }

    pub(crate) fn peer_check_version(bootstrap: cli::PeerBootstrapArgs) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "peer_check_version".to_string(),
            bootstrap: Some(bootstrap),
            ..Self::default()
        }
    }

    pub(crate) fn peer_rotate_token(bootstrap: cli::PeerBootstrapArgs) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "peer_rotate_token".to_string(),
            bootstrap: Some(bootstrap),
            ..Self::default()
        }
    }

    pub(crate) fn peer_forget(alias: String) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "peer_forget".to_string(),
            alias: Some(alias),
            ..Self::default()
        }
    }

    pub(crate) fn report(node: String, status: Value) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "report".to_string(),
            node: Some(node),
            status: Some(status),
            ..Self::default()
        }
    }

    pub(crate) fn to_line(&self) -> Result<String> {
        Ok(format!("{}\n", serde_json::to_string(self)?))
    }

    pub(crate) fn to_value(&self) -> Result<Value> {
        serde_json::to_value(self).context("failed to encode node control request")
    }

    pub(crate) fn validate_compatible(&self) -> Result<()> {
        if let Some(version) = self.api_version
            && version > NODE_CONTROL_VERSION
        {
            anyhow::bail!(
                "unsupported node control api_version {version}; local daemon supports {NODE_CONTROL_VERSION}"
            );
        }
        Ok(())
    }

    pub(crate) fn with_auth_token(mut self, token: Option<&str>) -> Self {
        if let Some(token) = token {
            self.auth_token = Some(token.to_string());
        }
        self
    }
}

pub(crate) fn attach_auth_token(value: &mut Value, token: Option<&str>) {
    let Some(token) = token else {
        return;
    };
    if let Value::Object(object) = value {
        object
            .entry("auth_token")
            .or_insert_with(|| Value::String(token.to_string()));
    }
}
