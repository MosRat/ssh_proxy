use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ssh_proxy_daemon::control::{
    NodeRequestIntent, NodeRequestKind, NodeRequestPayload, NodeRequestView,
};

use super::proxy_session::ProxySessionSpec;
use crate::{
    cli,
    protocol_core::{
        control::{DaemonControlCommand, DaemonControlPayloadShape},
        envelope::{ControlError, ControlResponse},
        version::CONTROL_API_VERSION,
    },
};

pub(crate) const NODE_CONTROL_VERSION: u16 = CONTROL_API_VERSION;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) node_scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) proxy_session: Option<ProxySessionSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) update_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) remote_url: Option<String>,
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

    pub(crate) fn node_ensure(scope: cli::NodeControlScope) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "node_ensure".to_string(),
            node_scope: Some(scope.as_str().to_string()),
            ..Self::default()
        }
    }

    pub(crate) fn node_start(id: String) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "node_start".to_string(),
            id: Some(id),
            ..Self::default()
        }
    }

    pub(crate) fn node_stop(id: String) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "node_stop".to_string(),
            id: Some(id),
            ..Self::default()
        }
    }

    pub(crate) fn node_restart(id: String) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "node_restart".to_string(),
            id: Some(id),
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

    pub(crate) fn peer_ensure(bootstrap: cli::PeerBootstrapArgs) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "peer_ensure".to_string(),
            bootstrap: Some(bootstrap),
            ..Self::default()
        }
    }

    pub(crate) fn peer_update(bootstrap: cli::PeerBootstrapArgs) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "peer_update".to_string(),
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

    pub(crate) fn ensure_proxy_session(proxy_session: ProxySessionSpec) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "ensure_proxy_session".to_string(),
            id: Some(proxy_session.job_id()),
            proxy_session: Some(proxy_session),
            ..Self::default()
        }
    }

    pub(crate) fn daemon_update(source: Option<String>) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "daemon_update".to_string(),
            update_source: source,
            ..Self::default()
        }
    }

    pub(crate) fn proxy_session_status(
        id: Option<String>,
        proxy_session: Option<ProxySessionSpec>,
    ) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "proxy_session_status".to_string(),
            id,
            proxy_session,
            ..Self::default()
        }
    }

    pub(crate) fn proxy_session_down(
        id: Option<String>,
        proxy_session: Option<ProxySessionSpec>,
    ) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "proxy_session_down".to_string(),
            id,
            proxy_session,
            ..Self::default()
        }
    }

    pub(crate) fn apply_remote_settings(
        target: String,
        workspace: String,
        remote_url: String,
    ) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "apply_remote_settings".to_string(),
            id: Some(workspace),
            alias: Some(target),
            remote_url: Some(remote_url),
            ..Self::default()
        }
    }

    pub(crate) fn job_events(id: Option<String>) -> Self {
        Self {
            api_version: Some(NODE_CONTROL_VERSION),
            cmd: "job_events".to_string(),
            id,
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

    pub(crate) fn command_kind(&self) -> DaemonControlCommand {
        DaemonControlCommand::parse(&self.cmd)
    }

    pub(crate) fn payload_shape(&self) -> DaemonControlPayloadShape {
        self.command_kind().payload_shape()
    }

    pub(crate) fn typed_payload(&self) -> NodeRequestPayload {
        match self.payload_shape() {
            DaemonControlPayloadShape::Empty => NodeRequestPayload::Empty,
            DaemonControlPayloadShape::Profile => NodeRequestPayload::Profile {
                profile: self.profile.clone(),
            },
            DaemonControlPayloadShape::Id => NodeRequestPayload::Id {
                id: self.id.clone(),
            },
            DaemonControlPayloadShape::RouteStart => NodeRequestPayload::RouteStart {
                id: self.id.clone(),
                direction: self.direction.clone(),
                persist: self.persist,
                has_proxy: self.proxy.is_some(),
                has_reverse: self.reverse.is_some(),
                connect_mode: self.connect_mode.clone(),
            },
            DaemonControlPayloadShape::RouteArgs => NodeRequestPayload::RouteArgs {
                has_route: self.route.is_some(),
            },
            DaemonControlPayloadShape::PeerBootstrap => NodeRequestPayload::PeerBootstrap {
                has_bootstrap: self.bootstrap.is_some(),
            },
            DaemonControlPayloadShape::Report => NodeRequestPayload::Report {
                node: self.node.clone(),
                has_status: self.status.is_some(),
            },
            DaemonControlPayloadShape::ProxySession => NodeRequestPayload::ProxySession {
                id: self.id.clone(),
                has_spec: self.proxy_session.is_some(),
            },
            DaemonControlPayloadShape::RemoteSettings => NodeRequestPayload::RemoteSettings {
                target: self.alias.clone(),
                workspace: self.id.clone(),
                remote_url: self.remote_url.clone(),
            },
            DaemonControlPayloadShape::DaemonUpdate => NodeRequestPayload::DaemonUpdate {
                source: self.update_source.clone(),
            },
            DaemonControlPayloadShape::JobEvents => NodeRequestPayload::JobEvents {
                id: self.id.clone(),
            },
            DaemonControlPayloadShape::Unknown => NodeRequestPayload::Unknown,
        }
    }

    pub(crate) fn typed_intent(&self) -> NodeRequestIntent {
        let command = self.command_kind();
        NodeRequestIntent::new(
            command.canonical_name(),
            self.api_version,
            self.id.clone(),
            self.alias.clone().or_else(|| self.node.clone()),
            self.typed_payload(),
        )
    }

    pub(crate) fn typed_view(&self) -> NodeRequestView {
        self.typed_intent().view()
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn node_response_uses_shared_control_shape() {
        let value = serde_json::to_value(NodeResponse::error("bad_request", "nope")).unwrap();

        assert_eq!(value["api_version"], NODE_CONTROL_VERSION);
        assert_eq!(value["ok"], false);
        assert_eq!(value["code"], "bad_request");
        assert_eq!(value["error"], "nope");
        assert!(value.get("data").is_none());
    }

    #[test]
    fn response_value_preserves_existing_object_payloads() {
        let value = response_value(json!({
            "ok": true,
            "kind": "status",
            "items": []
        }));

        assert_eq!(value["api_version"], NODE_CONTROL_VERSION);
        assert_eq!(value["kind"], "status");
        assert!(value.get("data").is_none());
    }

    #[test]
    fn response_value_wraps_legacy_non_object_payloads() {
        let value = response_value(json!(["route-1"]));

        assert_eq!(value["api_version"], NODE_CONTROL_VERSION);
        assert_eq!(value["ok"], true);
        assert_eq!(value["data"][0], "route-1");
    }

    #[test]
    fn legacy_node_request_json_gets_typed_payload_view() {
        let request: NodeRequest = serde_json::from_value(json!({
            "cmd": "route-start",
            "id": "route-1",
            "direction": "forward",
            "persist": true
        }))
        .unwrap();

        assert_eq!(request.command_kind(), DaemonControlCommand::RouteStart);
        assert_eq!(
            request.payload_shape(),
            DaemonControlPayloadShape::RouteStart
        );
        assert_eq!(
            request.typed_payload(),
            NodeRequestPayload::RouteStart {
                id: Some("route-1".to_string()),
                direction: Some("forward".to_string()),
                persist: Some(true),
                has_proxy: false,
                has_reverse: false,
                connect_mode: None,
            }
        );
        assert_eq!(request.typed_view().kind, NodeRequestKind::RouteStart);
        assert_eq!(request.typed_view().command, "route_start");
    }

    #[test]
    fn apply_settings_request_has_typed_payload_view() {
        let request = NodeRequest::apply_remote_settings(
            "target".to_string(),
            "workspace".to_string(),
            "http://127.0.0.1:17890".to_string(),
        );

        assert_eq!(
            request.typed_payload(),
            NodeRequestPayload::RemoteSettings {
                target: Some("target".to_string()),
                workspace: Some("workspace".to_string()),
                remote_url: Some("http://127.0.0.1:17890".to_string()),
            }
        );
        assert_eq!(
            request.typed_view(),
            NodeRequestView {
                kind: NodeRequestKind::ApplyRemoteSettings,
                command: "apply_remote_settings".to_string(),
                api_version: Some(NODE_CONTROL_VERSION),
                id: Some("workspace".to_string()),
                alias: Some("target".to_string()),
            }
        );
        assert_eq!(request.typed_intent().payload, request.typed_payload());
    }
}
