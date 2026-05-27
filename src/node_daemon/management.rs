use anyhow::{Result, anyhow};
use serde_json::json;

use super::{NodeManager, NodeRequest, NodeResponse, response_line};

impl NodeManager {
    pub(super) async fn nodes_json(&self) -> Result<String> {
        let instances = self.instances.lock().await;
        let profiles = instances
            .iter()
            .map(|(name, handle)| {
                json!({
                    "id": format!("profile:{name}"),
                    "name": name,
                    "scope": "user",
                    "state": if handle.is_finished() { "stopped" } else { "running" },
                    "managed_by": "current-daemon",
                    "kind": "profile-instance",
                })
            })
            .collect::<Vec<_>>();
        response_line(json!({
            "ok": true,
            "kind": "nodes",
            "broker_api": "v0.2",
            "nodes": [{
                "id": "current",
                "name": self.name,
                "scope": "current",
                "state": "running",
                "managed_by": "current-daemon",
                "control_endpoint": self.control_endpoint.to_string(),
                "transport": self.transport.map(|addr| addr.to_string()),
                "tls_transport": self.tls_transport.map(|addr| addr.to_string()),
                "quic_transport": self.quic_transport.map(|addr| addr.to_string()),
                "pid": std::process::id(),
                "capabilities": [
                    "route_intent",
                    "route_readiness",
                    "peer_ensure",
                    "peer_update",
                    "jobs",
                    "profile_instances"
                ],
            }],
            "profile_instances": profiles,
        }))
    }

    pub(super) async fn jobs_json(&self) -> Result<String> {
        response_line(json!({
            "ok": true,
            "kind": "jobs",
            "broker_api": "v0.2",
            "jobs": [],
            "message": "long-running broker jobs are exposed through this allowlisted endpoint",
        }))
    }

    pub(super) async fn node_ensure(&self, request: NodeRequest) -> Result<String> {
        let scope = request.node_scope.as_deref().unwrap_or("user");
        let state = if scope == "session" {
            "session-ready"
        } else {
            "running"
        };
        response_line(json!({
            "ok": true,
            "kind": "node_ensure",
            "broker_api": "v0.2",
            "changed": false,
            "requested_scope": scope,
            "node": {
                "id": "current",
                "name": self.name,
                "scope": "current",
                "state": state,
                "control_endpoint": self.control_endpoint.to_string(),
            },
            "next_action": "reuse_current_daemon",
        }))
    }

    pub(super) async fn node_start(&self, request: NodeRequest) -> Result<String> {
        let id = request
            .id
            .ok_or_else(|| anyhow!("node_start requires id"))?;
        if is_current_node_id(&id) {
            return response_line(json!({
                "ok": true,
                "kind": "node_start",
                "changed": false,
                "id": id,
                "state": "running",
                "message": "current node daemon is already running",
            }));
        }
        response_line(json!({
            "ok": false,
            "kind": "node_start",
            "code": "unknown_node",
            "id": id,
            "state": "unavailable",
            "next_action": "node_ensure",
            "message": "this broker only manages the current daemon in v0.2 preview",
        }))
    }

    pub(super) async fn node_stop(&self, request: NodeRequest) -> Result<String> {
        let id = request.id.ok_or_else(|| anyhow!("node_stop requires id"))?;
        if is_current_node_id(&id) {
            return self.shutdown().await;
        }
        response_line(json!({
            "ok": false,
            "kind": "node_stop",
            "code": "unknown_node",
            "id": id,
            "next_action": "nodes",
        }))
    }

    pub(super) async fn node_restart(&self, request: NodeRequest) -> Result<String> {
        let id = request
            .id
            .ok_or_else(|| anyhow!("node_restart requires id"))?;
        if is_current_node_id(&id) {
            return response_line(json!({
                "ok": false,
                "kind": "node_restart",
                "code": "requires_supervisor",
                "id": id,
                "next_action": "service_ensure_or_session_daemon_restart",
                "message": "current daemon cannot restart itself without an external broker supervisor",
            }));
        }
        response_line(json!({
            "ok": false,
            "kind": "node_restart",
            "code": "unknown_node",
            "id": id,
            "next_action": "nodes",
        }))
    }

    pub(super) async fn ensure_peer(&self, request: NodeRequest) -> Result<String> {
        let Some(args) = request.bootstrap else {
            return NodeResponse::error("bad_request", "peer_ensure requires bootstrap args")
                .to_line();
        };
        let alias = args.alias.clone().unwrap_or_else(|| args.target.clone());
        if self.peer_is_recorded(&alias).await {
            return response_line(json!({
                "ok": true,
                "kind": "peer_ensure",
                "broker_api": "v0.2",
                "alias": alias,
                "changed": false,
                "state": "ready",
                "next_action": "reuse_recorded_peer",
            }));
        }
        let response = self.bootstrap_peer_from_args(args).await?;
        let mut value: serde_json::Value = serde_json::from_str(response.trim())?;
        if let Some(object) = value.as_object_mut() {
            object.insert("kind".to_string(), json!("peer_ensure"));
            object.insert("broker_api".to_string(), json!("v0.2"));
            object.insert("state".to_string(), json!("bootstrapped"));
            object.insert("requires_external_ssh".to_string(), json!(false));
        }
        response_line(value)
    }

    pub(super) async fn update_peer(&self, request: NodeRequest) -> Result<String> {
        let Some(args) = request.bootstrap else {
            return NodeResponse::error("bad_request", "peer_update requires bootstrap args")
                .to_line();
        };
        let alias = args.alias.clone().unwrap_or_else(|| args.target.clone());
        let response = self.refresh_peer_from_args(args).await?;
        let mut value: serde_json::Value = serde_json::from_str(response.trim())?;
        if let Some(object) = value.as_object_mut() {
            object.insert("kind".to_string(), json!("peer_update"));
            object.insert("broker_api".to_string(), json!("v0.2"));
            object.insert("alias".to_string(), json!(alias));
            object.insert("state".to_string(), json!("refreshed"));
            object.insert("requires_external_ssh".to_string(), json!(false));
        }
        response_line(value)
    }
}

fn is_current_node_id(id: &str) -> bool {
    matches!(id, "current" | "local" | "self")
}
