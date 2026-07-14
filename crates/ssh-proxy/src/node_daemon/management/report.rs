use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::node_daemon::{NodeManager, NodeRequest, NodeResponse, response_line};

impl NodeManager {
    pub(in crate::node_daemon) async fn nodes_json(&self) -> Result<String> {
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

    pub(in crate::node_daemon) async fn jobs_json(&self) -> Result<String> {
        let job_values = self.jobs.jobs_value().await;
        response_line(json!({
            "ok": true,
            "kind": "jobs",
            "daemon_api": "v0.3",
            "jobs": job_values,
            "message": "proxy session jobs are the stable v0.3 progress surface",
        }))
    }

    pub(in crate::node_daemon) async fn job_events_json(
        &self,
        request: NodeRequest,
    ) -> Result<String> {
        let events = self
            .jobs
            .events(request.id.as_deref())
            .await
            .into_iter()
            .map(|event| serde_json::to_value(event).unwrap_or_else(|_| json!({})))
            .collect::<Vec<_>>();
        response_line(json!({
            "ok": true,
            "kind": "job_events",
            "daemon_api": "v0.3",
            "job_id": request.id,
            "events": events,
        }))
    }

    pub(in crate::node_daemon) async fn job_status_json(
        &self,
        request: NodeRequest,
    ) -> Result<String> {
        let id = request
            .id
            .ok_or_else(|| anyhow!("job_status requires id"))?;
        let job = self.jobs.get(&id).await;
        let ok = job.is_some();
        let code = if ok { Value::Null } else { json!("not_found") };
        response_line(json!({
            "ok": ok,
            "kind": "job_status",
            "daemon_api": "v0.3",
            "job": job.map(|job| job.to_value()),
            "code": code,
        }))
    }

    pub(in crate::node_daemon) async fn node_ensure(&self, request: NodeRequest) -> Result<String> {
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

    pub(in crate::node_daemon) async fn node_start(&self, request: NodeRequest) -> Result<String> {
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

    pub(in crate::node_daemon) async fn node_stop(&self, request: NodeRequest) -> Result<String> {
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

    pub(in crate::node_daemon) async fn node_restart(
        &self,
        request: NodeRequest,
    ) -> Result<String> {
        let id = request
            .id
            .ok_or_else(|| anyhow!("node_restart requires id"))?;
        if is_current_node_id(&id) {
            return response_line(json!({
                "ok": false,
                "kind": "node_restart",
                "code": "requires_supervisor",
                "id": id,
                "next_action": "daemon_restart_or_reinstall",
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

    pub(in crate::node_daemon) async fn ensure_peer(&self, request: NodeRequest) -> Result<String> {
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

    pub(in crate::node_daemon) async fn update_peer(
        self: Arc<Self>,
        request: NodeRequest,
    ) -> Result<String> {
        let Some(args) = request.bootstrap else {
            return NodeResponse::error("bad_request", "peer_update requires bootstrap args")
                .to_line();
        };
        let alias = args.alias.clone().unwrap_or_else(|| args.target.clone());
        let mut response = NodeRequest::peer_update(args);
        response.alias = Some(alias);
        let mut line = self
            .accept_remote_peer_job(
                response,
                "peer_update",
                "remote_peer_update",
                "peer update accepted",
            )
            .await?;
        let mut value: serde_json::Value = serde_json::from_str(line.trim())?;
        if let Some(object) = value.as_object_mut() {
            object.insert("broker_api".to_string(), json!("v0.2"));
            object.insert("requires_external_ssh".to_string(), json!(false));
        }
        line = response_line(value)?;
        Ok(line)
    }
}

fn is_current_node_id(id: &str) -> bool {
    matches!(id, "current" | "local" | "self")
}
