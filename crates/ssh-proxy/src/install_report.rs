use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::repair;

pub(crate) fn append_install_event(
    log_path: &Path,
    install_id: &str,
    state: &str,
    phase: &str,
    message: &str,
    blocker: Option<&str>,
) -> Result<Value> {
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create install log dir {}", parent.display()))?;
    }
    let mut event = json!({
        "install_id": install_id,
        "state": state,
        "phase": phase,
        "message": message,
        "created_at_unix": now_unix(),
    });
    if let Some(blocker) = blocker {
        event["blocker"] = json!(blocker);
        event["repair_action"] = repair::action_value_for_blocker(blocker);
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .with_context(|| format!("failed to open install log {}", log_path.display()))?;
    writeln!(file, "{}", serde_json::to_string(&event)?)
        .with_context(|| format!("failed to write install log {}", log_path.display()))?;
    Ok(event)
}

pub(crate) fn install_log_path_for_pid(pid: u32) -> PathBuf {
    std::env::temp_dir().join(format!("ssh_proxy-daemon-install-{pid}.jsonl"))
}

pub(crate) fn install_report_from_log(log_path: &Path) -> Value {
    let events = read_install_events(log_path);
    let last = events.last().cloned().unwrap_or_else(|| {
        json!({
            "state": "unknown",
            "phase": "unknown",
            "message": "install worker did not write a structured event",
        })
    });
    let state = last
        .get("state")
        .cloned()
        .unwrap_or_else(|| json!("unknown"));
    json!({
        "ok": state == "healthy",
        "kind": "daemon_install",
        "daemon_api": "v0.3",
        "install_id": last.get("install_id").cloned().unwrap_or(Value::Null),
        "state": state,
        "phase": last.get("phase").cloned().unwrap_or_else(|| json!("unknown")),
        "message": last.get("message").cloned().unwrap_or(Value::Null),
        "blocker": last.get("blocker").cloned().unwrap_or(Value::Null),
        "repair_action": last.get("repair_action").cloned().unwrap_or(Value::Null),
        "health_check": health_check_from_events(&events),
        "log_path": log_path.display().to_string(),
        "events": events,
    })
}

pub(crate) fn install_report_for_current_process() -> Value {
    let log_path = install_log_path_for_pid(std::process::id());
    install_report_from_log(&log_path)
}

fn read_install_events(log_path: &Path) -> Vec<Value> {
    let Ok(text) = fs::read_to_string(log_path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

fn health_check_from_events(events: &[Value]) -> Value {
    let health = events.iter().rev().find(|event| {
        event.get("phase").and_then(Value::as_str) == Some("health_check")
            || event.get("phase").and_then(Value::as_str) == Some("healthy")
            || event.get("state").and_then(Value::as_str) == Some("healthy")
    });
    match health {
        Some(event) if event.get("state").and_then(Value::as_str) == Some("healthy") => json!({
            "state": "passed",
            "message": event.get("message").cloned().unwrap_or(Value::Null),
        }),
        Some(event) if event.get("state").and_then(Value::as_str) == Some("failed") => json!({
            "state": "failed",
            "message": event.get("message").cloned().unwrap_or(Value::Null),
            "blocker": event.get("blocker").cloned().unwrap_or(Value::Null),
        }),
        Some(event) => json!({
            "state": "running",
            "message": event.get("message").cloned().unwrap_or(Value::Null),
        }),
        None => json!({
            "state": "unknown",
        }),
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_report_uses_last_event() {
        let dir = std::env::temp_dir().join(format!(
            "ssh_proxy-install-report-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("install.jsonl");
        append_install_event(&log, "install-test", "running", "prepare", "prepare", None).unwrap();
        append_install_event(
            &log,
            "install-test",
            "failed",
            "health_check",
            "failed",
            Some("daemon_unavailable"),
        )
        .unwrap();
        let report = install_report_from_log(&log);
        assert_eq!(report["state"], "failed");
        assert_eq!(report["phase"], "health_check");
        assert_eq!(report["blocker"], "daemon_unavailable");
        assert_eq!(report["health_check"]["state"], "failed");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
