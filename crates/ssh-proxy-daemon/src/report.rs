use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ssh_proxy_core::repair;

pub const DAEMON_API_VERSION: &str = "v0.3";
pub const DAEMON_INSTALL_NEXT_ACTION: &str = "ssh_proxy daemon install --scope system --elevate";

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
            daemon_api: Some(DAEMON_API_VERSION.to_string()),
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
                "daemon_api": DAEMON_API_VERSION,
                "error": "failed to encode daemon report",
            })
        })
    }
}

pub fn daemon_unavailable_job_status(
    id: &str,
    kind: &str,
    state: &str,
    phase: &str,
    message: &str,
) -> Value {
    let blocker = (state == "blocked").then_some(phase);
    json!({
        "id": id,
        "kind": kind,
        "state": state,
        "phase": phase,
        "progress": 0,
        "blocker": blocker,
        "next_action": Value::Null,
        "repair_action": blocker.map(repair::action_value_for_blocker).unwrap_or(Value::Null),
        "last_error": Value::Null,
        "message": message,
    })
}

pub fn daemon_update_unavailable(source: Option<String>) -> Value {
    json!({
        "ok": false,
        "kind": "daemon_update",
        "daemon_api": DAEMON_API_VERSION,
        "job": daemon_unavailable_job_status(
            "self-update:pending",
            "self_update",
            "blocked",
            "daemon_unavailable",
            "daemon self-update requires the running daemon",
        ),
        "source": source,
        "requires_daemon": true,
        "requires_elevation": true,
        "blocker": "daemon_unavailable",
        "next_action": DAEMON_INSTALL_NEXT_ACTION,
    })
}

pub fn proxy_session_unavailable(spec: Value, job_id: &str) -> Value {
    json!({
        "ok": false,
        "kind": "proxy_session",
        "daemon_api": DAEMON_API_VERSION,
        "spec": spec,
        "job": daemon_unavailable_job_status(
            job_id,
            "ensure_proxy_session",
            "blocked",
            "daemon_unavailable",
            "install or start the ssh_proxy daemon, then retry this proxy session",
        ),
        "blocker": "daemon_unavailable",
        "next_action": DAEMON_INSTALL_NEXT_ACTION,
        "retry_after_ms": 1000,
        "requires_daemon": true,
        "requires_elevation": true,
    })
}

pub fn proxy_session_down_unavailable(route_id: String) -> Value {
    json!({
        "ok": false,
        "kind": "proxy_session_down",
        "daemon_api": DAEMON_API_VERSION,
        "route_id": route_id,
        "code": "daemon_unavailable",
        "blocker": "daemon_unavailable",
        "next_action": DAEMON_INSTALL_NEXT_ACTION,
        "retry_after_ms": 1000,
        "requires_daemon": true,
        "requires_elevation": true,
    })
}

pub fn daemon_status_unavailable(
    version: &str,
    target: Option<String>,
    workspace: Option<String>,
) -> Value {
    json!({
        "ok": false,
        "kind": "daemon_status",
        "daemon_api": DAEMON_API_VERSION,
        "version": version,
        "target": target,
        "workspace": workspace,
        "health": "unavailable",
        "code": "daemon_unavailable",
        "blocker": "daemon_unavailable",
        "requires_elevation": true,
        "next_action": DAEMON_INSTALL_NEXT_ACTION,
        "retry_after_ms": 1000,
    })
}

pub fn daemon_events_unavailable(job: Option<String>) -> Value {
    json!({
        "ok": false,
        "kind": "daemon_events",
        "daemon_api": DAEMON_API_VERSION,
        "job": job,
        "events": [],
        "code": "daemon_unavailable",
        "blocker": "daemon_unavailable",
        "next_action": DAEMON_INSTALL_NEXT_ACTION,
        "retry_after_ms": 1000,
        "requires_daemon": true,
    })
}

pub fn vscode_apply_settings_unavailable(
    target: String,
    workspace: String,
    proxy_url: String,
) -> Value {
    json!({
        "ok": false,
        "kind": "vscode_apply_settings",
        "daemon_api": DAEMON_API_VERSION,
        "target": target,
        "workspace": workspace,
        "proxy_url": proxy_url,
        "job": daemon_unavailable_job_status(
            "vscode-settings:blocked",
            "apply_remote_settings",
            "blocked",
            "daemon_unavailable",
            "remote settings application requires the running daemon",
        ),
        "blocker": "daemon_unavailable",
        "next_action": DAEMON_INSTALL_NEXT_ACTION,
        "requires_daemon": true,
        "retry_after_ms": 1000,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_session_unavailable_preserves_legacy_fields() {
        let value = proxy_session_unavailable(json!({"target": "edge"}), "session:edge");

        assert_eq!(value["ok"], false);
        assert_eq!(value["kind"], "proxy_session");
        assert_eq!(value["daemon_api"], DAEMON_API_VERSION);
        assert_eq!(value["blocker"], "daemon_unavailable");
        assert_eq!(value["job"]["id"], "session:edge");
        assert_eq!(
            value["job"]["repair_action"]["command"],
            DAEMON_INSTALL_NEXT_ACTION
        );
    }

    #[test]
    fn status_unavailable_keeps_retry_hint() {
        let value = daemon_status_unavailable("0.1.1", Some("edge".to_string()), None);

        assert_eq!(value["health"], "unavailable");
        assert_eq!(value["retry_after_ms"], 1000);
    }
}
