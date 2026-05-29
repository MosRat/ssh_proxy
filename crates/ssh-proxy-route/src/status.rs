use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteStats {
    pub state: String,
    pub attempts: u64,
    pub restart_count: u64,
    pub last_error: Option<String>,
    pub last_event: Option<String>,
    pub started_at_unix: u64,
    pub updated_at_unix: u64,
}

impl Default for RouteStats {
    fn default() -> Self {
        let now = now_unix();
        Self {
            state: "starting".to_string(),
            attempts: 0,
            restart_count: 0,
            last_error: None,
            last_event: Some("route task created".to_string()),
            started_at_unix: now,
            updated_at_unix: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteTaskRecord {
    pub id: String,
    pub direction: String,
    pub detail: String,
    pub listen: Option<String>,
    pub peer: Option<String>,
    pub persist: bool,
    pub created_at_unix: u64,
    pub fallback_reason: Option<String>,
    pub task_finished: bool,
    pub runtime: Value,
    pub stats: RouteStats,
    pub link: Value,
}

impl RouteTaskRecord {
    pub fn to_status_report(&self) -> RouteStatusReport {
        RouteStatusReport {
            id: self.id.clone(),
            direction: self.direction.clone(),
            detail: self.detail.clone(),
            listen: self.listen.clone(),
            peer: self.peer.clone(),
            persist: self.persist,
            created_at_unix: self.created_at_unix,
            fallback_reason: self.fallback_reason.clone(),
            task_finished: self.task_finished,
            runtime: self.runtime.clone(),
            state: self.stats.state.clone(),
            last_error: self.stats.last_error.clone(),
            started_at: self.stats.started_at_unix,
            updated_at: self.stats.updated_at_unix,
            readiness: RouteReadinessReport::from_stats(&self.id, self.peer.clone(), &self.stats),
            managed_by: "current-daemon".to_string(),
            job_id: format!("route:{}", self.id),
            stats: self.stats.clone(),
            link: self.link.clone(),
        }
    }

    pub fn to_value(&self) -> Value {
        self.to_status_report().to_value()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteStatusReport {
    pub id: String,
    pub direction: String,
    pub detail: String,
    pub listen: Option<String>,
    pub peer: Option<String>,
    pub persist: bool,
    pub created_at_unix: u64,
    pub fallback_reason: Option<String>,
    pub task_finished: bool,
    pub runtime: Value,
    pub state: String,
    pub last_error: Option<String>,
    pub started_at: u64,
    pub updated_at: u64,
    pub readiness: RouteReadinessReport,
    pub managed_by: String,
    pub job_id: String,
    pub stats: RouteStats,
    pub link: Value,
}

impl RouteStatusReport {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteReadinessReport {
    pub state: String,
    pub phase: String,
    pub retry_count: u64,
    pub attempts: u64,
    pub blocker: Option<String>,
    pub next_action: String,
    pub managed_by: String,
    pub job_id: String,
    pub route_id: String,
    pub peer: Option<String>,
    pub updated_at: u64,
}

impl RouteReadinessReport {
    pub fn from_stats(id: &str, peer: Option<String>, stats: &RouteStats) -> Self {
        let phase = match stats.state.as_str() {
            "running" => "ready",
            "failed" | "error" => "failed",
            "restarting" => "starting",
            "stopping" | "stopped" => "stopped",
            _ => "starting",
        };
        let next_action = match phase {
            "ready" => "none",
            "failed" => "restart-route",
            "stopped" => "remove-or-restart-route",
            _ => "wait",
        };
        Self {
            state: stats.state.clone(),
            phase: phase.to_string(),
            retry_count: stats.restart_count,
            attempts: stats.attempts,
            blocker: stats.last_error.clone(),
            next_action: next_action.to_string(),
            managed_by: "current-daemon".to_string(),
            job_id: format!("route:{id}"),
            route_id: id.to_string(),
            peer,
            updated_at: stats.updated_at_unix,
        }
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn route_status_report_preserves_public_shape() {
        let stats = RouteStats {
            state: "failed".to_string(),
            attempts: 2,
            restart_count: 1,
            last_error: Some("boom".to_string()),
            last_event: Some("failed".to_string()),
            started_at_unix: 10,
            updated_at_unix: 20,
        };
        let record = RouteTaskRecord {
            id: "route-1".to_string(),
            direction: "forward".to_string(),
            detail: "127.0.0.1:8080 -> remote".to_string(),
            listen: Some("127.0.0.1:8080".to_string()),
            peer: Some("remote".to_string()),
            persist: true,
            created_at_unix: 1,
            fallback_reason: None,
            task_finished: false,
            runtime: json!({"selected_transport": "ssh-native"}),
            stats,
            link: Value::Null,
        };

        let value = record.to_value();

        assert_eq!(value["id"], "route-1");
        assert_eq!(value["readiness"]["phase"], "failed");
        assert_eq!(value["readiness"]["next_action"], "restart-route");
        assert_eq!(value["runtime"]["selected_transport"], "ssh-native");
        assert_eq!(value["stats"]["restart_count"], 1);
    }
}
