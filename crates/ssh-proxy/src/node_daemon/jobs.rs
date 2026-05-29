use std::{collections::BTreeMap, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
pub(super) use ssh_proxy_daemon::job::{
    DaemonJobEvent as JobEvent, DaemonJobPhase as JobPhase, DaemonJobRecord as JobRecord,
    DaemonJobState as JobState, now_unix,
};
use tokio::sync::Mutex;
use tracing::warn;

use crate::config;

const JOB_STORE_VERSION: u32 = 1;
const MAX_EVENTS: usize = 256;
const MAX_TERMINAL_JOBS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JobStore {
    version: u32,
    jobs: BTreeMap<String, JobRecord>,
    events: Vec<JobEvent>,
}

impl Default for JobStore {
    fn default() -> Self {
        Self {
            version: JOB_STORE_VERSION,
            jobs: BTreeMap::new(),
            events: Vec::new(),
        }
    }
}

pub(super) struct JobRegistry {
    path: PathBuf,
    store: Mutex<JobStore>,
}

impl JobRegistry {
    pub(super) fn load(path: PathBuf) -> Self {
        let store = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|err| {
                warn!(path = %path.display(), error = %err, "failed to parse daemon job store");
                JobStore::default()
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => JobStore::default(),
            Err(err) => {
                warn!(path = %path.display(), error = %err, "failed to read daemon job store");
                JobStore::default()
            }
        };
        Self {
            path,
            store: Mutex::new(store),
        }
    }

    pub(super) async fn upsert(
        &self,
        record: JobRecord,
        message: impl Into<String>,
    ) -> Result<JobRecord> {
        let mut store = self.store.lock().await;
        store.jobs.insert(record.id.clone(), record.clone());
        push_event(&mut store, &record, message.into());
        prune_store(&mut store);
        save_store(&self.path, &store)?;
        Ok(record)
    }

    pub(super) async fn get(&self, id: &str) -> Option<JobRecord> {
        self.store.lock().await.jobs.get(id).cloned()
    }

    pub(super) async fn list(&self) -> Vec<JobRecord> {
        self.store.lock().await.jobs.values().cloned().collect()
    }

    pub(super) async fn events(&self, job_id: Option<&str>) -> Vec<JobEvent> {
        self.store
            .lock()
            .await
            .events
            .iter()
            .filter(|event| job_id.is_none_or(|id| event.job_id == id))
            .cloned()
            .collect()
    }

    pub(super) async fn jobs_value(&self) -> Value {
        let jobs = self
            .list()
            .await
            .into_iter()
            .map(|job| job.to_value())
            .collect::<Vec<_>>();
        json!(jobs)
    }
}

pub(super) fn daemon_status_block(
    status: &Value,
    jobs: &[JobRecord],
    daemon_state: Option<&Value>,
) -> Value {
    let update_state = daemon_state
        .and_then(|state| state.get("update").cloned())
        .or_else(|| {
            daemon_state
                .and_then(|state| state.get("update_state"))
                .cloned()
                .map(|state| {
                    json!({
                        "state": state,
                        "last_job": latest_update_job(jobs),
                    })
                })
        })
        .unwrap_or_else(|| {
            json!({
                "state": "idle",
                "last_job": latest_update_job(jobs),
            })
        });
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
        "jobs_summary": jobs_summary(jobs),
        "update_state": update_state,
    })
}

fn latest_update_job(jobs: &[JobRecord]) -> Value {
    jobs.iter()
        .filter(|job| job.kind == "self_update")
        .max_by_key(|job| job.updated_at_unix)
        .map(JobRecord::to_value)
        .unwrap_or(Value::Null)
}

fn jobs_summary(jobs: &[JobRecord]) -> Value {
    let mut queued = 0;
    let mut running = 0;
    let mut healthy = 0;
    let mut failed = 0;
    for job in jobs {
        match job.state {
            JobState::Queued => queued += 1,
            JobState::Running | JobState::WaitingRetry => running += 1,
            JobState::Healthy => healthy += 1,
            JobState::Failed => failed += 1,
            JobState::Cancelled => {}
        }
    }
    json!({
        "total": jobs.len(),
        "queued": queued,
        "running": running,
        "healthy": healthy,
        "failed": failed,
    })
}

fn push_event(store: &mut JobStore, record: &JobRecord, message: String) {
    store.events.push(JobEvent {
        job_id: record.id.clone(),
        state: record.state,
        phase: record.phase,
        message,
        created_at_unix: now_unix(),
    });
    if store.events.len() > MAX_EVENTS {
        let excess = store.events.len() - MAX_EVENTS;
        store.events.drain(0..excess);
    }
}

fn prune_store(store: &mut JobStore) {
    let terminal = store
        .jobs
        .values()
        .filter(|job| job.state.is_terminal())
        .count();
    if terminal <= MAX_TERMINAL_JOBS {
        return;
    }
    let mut terminal_ids = store
        .jobs
        .values()
        .filter(|job| job.state.is_terminal())
        .map(|job| (job.updated_at_unix, job.id.clone()))
        .collect::<Vec<_>>();
    terminal_ids.sort_by_key(|(updated, _)| *updated);
    for (_, id) in terminal_ids.into_iter().take(terminal - MAX_TERMINAL_JOBS) {
        store.jobs.remove(&id);
    }
}

fn save_store(path: &PathBuf, store: &JobStore) -> Result<()> {
    let text = serde_json::to_string_pretty(store).context("failed to encode daemon job store")?;
    config::save_text_file_private(path, &text)
        .with_context(|| format!("failed to save daemon job store {}", path.display()))
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
    fn job_record_transitions_keep_timestamps_monotonic() {
        let queued = JobRecord::new("job-1", "ensure_proxy_session");
        let running = queued
            .clone()
            .transition(JobState::Running, JobPhase::EnsurePeer, 25);
        assert_eq!(running.state, JobState::Running);
        assert_eq!(running.phase, JobPhase::EnsurePeer);
        assert_eq!(running.progress, 25);
        assert!(running.updated_at_unix >= queued.created_at_unix);
    }

    #[test]
    fn job_store_prunes_old_terminal_jobs() {
        let mut store = JobStore::default();
        for index in 0..(MAX_TERMINAL_JOBS + 2) {
            let mut job = JobRecord::new(format!("job-{index}"), "test");
            job.state = JobState::Healthy;
            job.updated_at_unix = index as u64;
            store.jobs.insert(job.id.clone(), job);
        }
        prune_store(&mut store);
        assert_eq!(store.jobs.len(), MAX_TERMINAL_JOBS);
        assert!(!store.jobs.contains_key("job-0"));
    }

    #[test]
    fn event_ring_filters_by_job() {
        let mut store = JobStore::default();
        let left = JobRecord::new("left", "test");
        let right = JobRecord::new("right", "test");
        push_event(&mut store, &left, "left queued".to_string());
        push_event(&mut store, &right, "right queued".to_string());
        let filtered = store
            .events
            .iter()
            .filter(|event| event.job_id == "left")
            .collect::<Vec<_>>();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].message, "left queued");
    }

    #[test]
    fn waiting_retry_jobs_serialize_retry_after_ms() {
        let waiting = JobRecord::new("proxy:window", "ensure_proxy_session")
            .transition(JobState::WaitingRetry, JobPhase::VerifyRemotePort, 85)
            .with_retry_after_ms(250);
        let value = waiting.to_value();
        assert_eq!(value["state"], "waiting_retry");
        assert_eq!(value["phase"], "verify_remote_port");
        assert_eq!(value["retry_after_ms"], 250);

        let healthy = waiting.transition(JobState::Healthy, JobPhase::Healthy, 100);
        assert_eq!(healthy.to_value().get("retry_after_ms"), None);
    }

    #[test]
    fn daemon_status_uses_persisted_update_state() {
        let job = JobRecord::new("self-update:pending", "self_update").transition(
            JobState::WaitingRetry,
            JobPhase::RestartDaemon,
            80,
        );
        let status = daemon_status_block(
            &json!({
                "ok": true,
                "control": "npipe://ssh_proxy/system/control",
            }),
            &[job],
            Some(&json!({
                "update_state": "restart_daemon",
                "update": {
                    "state": "restart_daemon",
                    "staged_version": "0.2.0",
                }
            })),
        );
        assert_eq!(status["update_state"]["state"], "restart_daemon");
        assert_eq!(status["update_state"]["staged_version"], "0.2.0");
    }
}
