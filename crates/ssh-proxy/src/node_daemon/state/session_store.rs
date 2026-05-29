use anyhow::Result;
use serde_json::{Value, json};

use super::{
    HandoffProbeStatus, JobRecord, ProductionState, ProxySessionRecord, ProxySessionSpec,
    RemoteSetupStatus, file_store::save_store, now_unix, proxy_session_record_from_spec_and_job,
    update_proxy_session_record_from_job,
};

impl ProductionState {
    pub(in crate::node_daemon) async fn upsert_session_from_job(
        &self,
        spec: &ProxySessionSpec,
        job: &JobRecord,
        route: Option<Value>,
    ) -> Result<ProxySessionRecord> {
        let mut store = self.sessions.lock().await;
        let session_id = spec.session_id();
        let entry = store
            .sessions
            .entry(session_id.clone())
            .or_insert_with(|| proxy_session_record_from_spec_and_job(spec, job));
        update_proxy_session_record_from_job(entry, spec, job);
        if route.is_some() {
            entry.route = route;
        }
        let record = entry.clone();
        save_store(&self.sessions_path, &*store)?;
        Ok(record)
    }

    pub(in crate::node_daemon) async fn cancel_session(
        &self,
        route_id: &str,
        job_id: &str,
        error: Option<String>,
    ) -> Result<Option<ProxySessionRecord>> {
        let mut store = self.sessions.lock().await;
        let mut found = None;
        for record in store.sessions.values_mut() {
            if record.route_id == route_id || record.job_id == job_id {
                record.state = "cancelled".to_string();
                record.phase = "cancelled".to_string();
                record.health = "cancelled".to_string();
                record.last_error = error.clone();
                record.updated_at_unix = now_unix();
                found = Some(record.clone());
                break;
            }
        }
        save_store(&self.sessions_path, &*store)?;
        Ok(found)
    }

    pub(in crate::node_daemon) async fn update_remote_setup_status(
        &self,
        session_id: &str,
        job_id: &str,
        status: RemoteSetupStatus,
    ) -> Result<Option<ProxySessionRecord>> {
        let mut store = self.sessions.lock().await;
        let mut found = None;
        for record in store.sessions.values_mut() {
            if record.session_id == session_id || record.job_id == job_id {
                record.remote_setup = status.clone();
                record.updated_at_unix = now_unix();
                found = Some(record.clone());
                break;
            }
        }
        save_store(&self.sessions_path, &*store)?;
        Ok(found)
    }

    pub(in crate::node_daemon) async fn update_handoff_probe_status(
        &self,
        session_id: &str,
        job_id: &str,
        status: HandoffProbeStatus,
    ) -> Result<Option<ProxySessionRecord>> {
        let mut store = self.sessions.lock().await;
        let mut found = None;
        for record in store.sessions.values_mut() {
            if record.session_id == session_id || record.job_id == job_id {
                record.handoff_probe = Some(serde_json::to_value(status).unwrap_or(Value::Null));
                record.updated_at_unix = now_unix();
                found = Some(record.clone());
                break;
            }
        }
        save_store(&self.sessions_path, &*store)?;
        Ok(found)
    }

    pub(in crate::node_daemon) async fn session_by_job(
        &self,
        job_id: &str,
    ) -> Option<ProxySessionRecord> {
        self.sessions
            .lock()
            .await
            .sessions
            .values()
            .find(|session| session.job_id == job_id)
            .cloned()
    }

    pub(in crate::node_daemon) async fn session_by_route(
        &self,
        route_id: &str,
    ) -> Option<ProxySessionRecord> {
        self.sessions
            .lock()
            .await
            .sessions
            .values()
            .find(|session| session.route_id == route_id)
            .cloned()
    }

    pub(in crate::node_daemon) async fn sessions_value(&self) -> Value {
        let sessions = self
            .sessions
            .lock()
            .await
            .sessions
            .values()
            .map(ProxySessionRecord::to_value)
            .collect::<Vec<_>>();
        json!(sessions)
    }

    pub(in crate::node_daemon) async fn unfinished_sessions(&self) -> Vec<ProxySessionRecord> {
        self.sessions
            .lock()
            .await
            .sessions
            .values()
            .filter(|session| !matches!(session.state.as_str(), "healthy" | "failed" | "cancelled"))
            .cloned()
            .collect()
    }
}
