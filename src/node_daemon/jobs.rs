use serde_json::{Value, json};

pub(super) fn route_jobs_from_status(status: &Value) -> Vec<Value> {
    status
        .get("routes")
        .and_then(Value::as_array)
        .map(|routes| routes.iter().map(route_job_value).collect())
        .unwrap_or_default()
}

pub(super) fn route_job_value(route: &Value) -> Value {
    let route_id = string_field(route, "id").unwrap_or_else(|| "unknown-route".to_string());
    let readiness = route.get("readiness").unwrap_or(&Value::Null);
    let state = string_field(readiness, "state")
        .or_else(|| string_field(route, "state"))
        .unwrap_or_else(|| "unknown".to_string());
    let phase = string_field(readiness, "phase")
        .or_else(|| string_field(route, "state"))
        .unwrap_or_else(|| state.clone());
    let last_error = string_field(route, "last_error")
        .or_else(|| string_field(readiness, "last_error"))
        .or_else(|| {
            route
                .pointer("/stats/last_error")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    json!({
        "id": string_field(route, "job_id").unwrap_or_else(|| format!("route:{route_id}")),
        "kind": "route",
        "state": job_state(&state),
        "phase": phase,
        "progress": job_progress(&state),
        "blocker": readiness.get("blocker").cloned().unwrap_or(Value::Null),
        "next_action": readiness.get("next_action").cloned().unwrap_or(Value::Null),
        "last_error": last_error,
        "route_id": route_id,
        "target": string_field(route, "peer"),
        "managed_by": string_field(route, "managed_by").unwrap_or_else(|| "daemon".to_string()),
        "updated_at": route.get("updated_at").cloned().unwrap_or(Value::Null),
    })
}

pub(super) fn daemon_status_block(status: &Value) -> Value {
    json!({
        "api": "v0.3",
        "version": env!("CARGO_PKG_VERSION"),
        "endpoint": status.get("control").cloned().unwrap_or(Value::Null),
        "scope": "daemon",
        "privilege": privilege_boundary(),
        "health": if status.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            "healthy"
        } else {
            "degraded"
        },
        "update_state": {
            "state": "idle",
            "last_job": Value::Null,
        },
    })
}

fn job_state(state: &str) -> &'static str {
    match state {
        "ready" | "running" => "running",
        "accepted" | "starting" | "bootstrapping_peer" => "accepted",
        "failed" | "error" => "failed",
        "exited" | "stopped" => "completed",
        "restarting" => "running",
        _ => "unknown",
    }
}

fn job_progress(state: &str) -> u8 {
    match state {
        "ready" | "running" => 100,
        "failed" | "error" => 100,
        "accepted" => 10,
        "starting" | "bootstrapping_peer" => 35,
        "restarting" => 60,
        _ => 0,
    }
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn privilege_boundary() -> Value {
    json!({
        "arbitrary_shell": false,
        "allowlisted_jobs": [
            "self_update",
            "remote_peer_update",
            "ensure_proxy_session",
            "apply_remote_settings",
            "route_lifecycle"
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_job_maps_readiness_to_job_status() {
        let job = route_job_value(&json!({
            "id": "v3-window",
            "job_id": "route:v3-window",
            "peer": "126",
            "state": "running",
            "readiness": {
                "state": "ready",
                "phase": "remote_verify",
                "next_action": "none"
            },
            "managed_by": "current-daemon"
        }));
        assert_eq!(job["id"], "route:v3-window");
        assert_eq!(job["state"], "running");
        assert_eq!(job["progress"], 100);
        assert_eq!(job["target"], "126");
    }
}
