use anyhow::Result;
use serde_json::{Value, json};

use super::{ProxySessionSpec, state_machine};
use crate::node_daemon::{jobs::JobRecord, response_line};

pub(super) fn accepted_response(
    spec: &ProxySessionSpec,
    job: &JobRecord,
    session: Value,
    reused_existing: bool,
) -> Result<String> {
    response_line(json!({
        "ok": true,
        "kind": "proxy_session",
        "daemon_api": "v0.3",
        "accepted": true,
        "reused_existing": reused_existing,
        "session_id": spec.session_id(),
        "job": job.to_value(),
        "session": session,
        "spec": spec.to_value(),
        "route": {
            "route_id": spec.route_id(),
            "remote_url": spec.remote_url(),
            "readiness": {
                "state": if reused_existing { "reused" } else { "accepted" },
                "phase": if reused_existing { "existing_job" } else { "queued" },
                "next_action": "poll_job"
            }
        },
        "remote_url": spec.remote_url(),
        "apply_remote_settings_required": true,
    }))
}

pub(super) fn find_route(status: &Value, route_id: &str) -> Option<Value> {
    status
        .get("routes")
        .and_then(Value::as_array)?
        .iter()
        .find(|route| route.get("id").and_then(Value::as_str) == Some(route_id))
        .cloned()
}

pub(super) fn route_state(route: &Value) -> Option<String> {
    route
        .pointer("/readiness/state")
        .and_then(Value::as_str)
        .or_else(|| route.get("state").and_then(Value::as_str))
        .map(str::to_string)
}

pub(super) fn route_from_job(job: &JobRecord) -> Value {
    json!({
        "route_id": job.route_id,
        "remote_url": job.remote_url,
        "health": state_machine::job_health(job),
    })
}

pub(super) fn missing_route(route_id: Option<String>, remote_url: Option<String>) -> Value {
    json!({
        "route_id": route_id,
        "remote_url": remote_url,
        "health": "starting",
        "readiness": {
            "state": "missing",
            "phase": "reconciling",
            "blocker": "route_not_running",
            "next_action": "rerun_ensure_proxy_session",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_daemon::jobs::{JobPhase, JobState};

    #[test]
    fn status_helpers_extract_route_readiness_state() {
        let status = json!({
            "routes": [{
                "id": "v3-window-a",
                "state": "running",
                "readiness": {
                    "state": "healthy"
                }
            }]
        });

        let route = find_route(&status, "v3-window-a").expect("route");

        assert_eq!(route_state(&route).as_deref(), Some("healthy"));
        assert!(find_route(&status, "missing").is_none());
    }

    #[test]
    fn route_from_job_uses_shared_job_health() {
        let job = JobRecord::new("proxy:window-a", "ensure_proxy_session").transition(
            JobState::WaitingRetry,
            JobPhase::EnsureTransport,
            45,
        );

        let route = route_from_job(&job);

        assert_eq!(route["health"], "starting");
    }
}
