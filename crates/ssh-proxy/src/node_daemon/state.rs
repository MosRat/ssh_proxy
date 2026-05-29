use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use ssh_proxy_daemon::job::{enum_value, job_health};
pub(super) use ssh_proxy_daemon::{
    DaemonStateRecord, PeerStatusRecord, ProxySessionRecord, RemoteSetupStatus,
};
use tokio::sync::Mutex;

use crate::config;

use super::{
    handoff::HandoffProbeStatus,
    jobs::JobRecord,
    proxy_session::{ProxySessionSpec, RemotePortPolicy},
};

const STORE_VERSION: u32 = 1;

mod daemon_store;
mod file_store;
mod peer_store;
mod session_store;

use file_store::load_store;

pub(super) trait ProxySessionRecordExt {
    fn to_spec(&self) -> Result<ProxySessionSpec>;
}

impl ProxySessionRecordExt for ProxySessionRecord {
    fn to_spec(&self) -> Result<ProxySessionSpec> {
        Ok(ProxySessionSpec {
            target: self.target.clone(),
            workspace_id: self.workspace_id.clone(),
            ssh: decode_optional(&self.ssh)?,
            workspace_paths: self.workspace_paths.clone(),
            local_proxy: self.local_proxy.clone(),
            remote_bind: self.remote_bind.parse()?,
            remote_port_policy: RemotePortPolicy {
                preferred: self.remote_port,
                auto_pick: true,
            },
            connect_mode: crate::cli::RouteConnectMode::ReverseLink,
            apply_policy: decode_or_default(&self.apply_policy)?,
        })
    }
}

fn proxy_session_record_from_spec_and_job(
    spec: &ProxySessionSpec,
    job: &JobRecord,
) -> ProxySessionRecord {
    ProxySessionRecord {
        session_id: spec.session_id(),
        target: spec.target.clone(),
        workspace_id: spec.workspace_id.clone(),
        ssh: encode_optional(&spec.ssh),
        workspace_paths: spec.workspace_paths.clone(),
        job_id: job.id.clone(),
        route_id: spec.route_id(),
        local_proxy: spec.local_proxy.clone(),
        remote_bind: spec.remote_bind.to_string(),
        remote_port: spec.remote_port_policy.preferred,
        remote_url: job.remote_url.clone().unwrap_or_else(|| spec.remote_url()),
        apply_policy: encode_some(&spec.apply_policy),
        state: enum_value(&job.state),
        phase: enum_value(&job.phase),
        health: job_health(job).to_string(),
        blocker: job.blocker.clone(),
        next_action: job.next_action.clone(),
        repair_action: job.repair_action.clone(),
        last_error: job.last_error.clone(),
        remote_setup: RemoteSetupStatus::pending(),
        handoff_probe: None,
        recovery_attempts: job.recovery_attempts,
        route: None,
        created_at_unix: job.created_at_unix,
        updated_at_unix: job.updated_at_unix,
    }
}

fn update_proxy_session_record_from_job(
    record: &mut ProxySessionRecord,
    spec: &ProxySessionSpec,
    job: &JobRecord,
) {
    record.target = spec.target.clone();
    record.workspace_id = spec.workspace_id.clone();
    record.ssh = encode_optional(&spec.ssh);
    record.workspace_paths = spec.workspace_paths.clone();
    record.route_id = spec.route_id();
    record.local_proxy = spec.local_proxy.clone();
    record.remote_bind = spec.remote_bind.to_string();
    record.remote_port = spec.remote_port_policy.preferred;
    record.remote_url = job.remote_url.clone().unwrap_or_else(|| spec.remote_url());
    record.apply_policy = encode_some(&spec.apply_policy);
    record.state = enum_value(&job.state);
    record.phase = enum_value(&job.phase);
    record.health = job_health(job).to_string();
    record.blocker = job.blocker.clone();
    record.next_action = job.next_action.clone();
    record.repair_action = job.repair_action.clone();
    record.last_error = job.last_error.clone();
    record.updated_at_unix = job.updated_at_unix;
    if record.phase == "apply_remote_settings" {
        record.remote_setup = RemoteSetupStatus::required();
        record.remote_setup.updated_at_unix = job.updated_at_unix;
    }
    if record.phase == "healthy" && record.remote_setup.state == "pending" {
        record.remote_setup = RemoteSetupStatus::required();
        record.remote_setup.updated_at_unix = job.updated_at_unix;
    }
    record.recovery_attempts = job.recovery_attempts;
}

fn encode_optional<T: Serialize>(value: &Option<T>) -> Option<Value> {
    value
        .as_ref()
        .and_then(|value| serde_json::to_value(value).ok())
}

fn encode_some<T: Serialize>(value: &T) -> Option<Value> {
    serde_json::to_value(value).ok()
}

fn decode_optional<T: DeserializeOwned>(value: &Option<Value>) -> Result<Option<T>> {
    value
        .as_ref()
        .map(|value| serde_json::from_value(value.clone()).map_err(Into::into))
        .transpose()
}

fn decode_or_default<T: DeserializeOwned + Default>(value: &Option<Value>) -> Result<T> {
    value
        .as_ref()
        .map(|value| serde_json::from_value(value.clone()).map_err(Into::into))
        .transpose()
        .map(|value| value.unwrap_or_default())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionStore {
    version: u32,
    sessions: BTreeMap<String, ProxySessionRecord>,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            sessions: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PeerStore {
    version: u32,
    peers: BTreeMap<String, PeerStatusRecord>,
}

impl Default for PeerStore {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            peers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonStateStore {
    version: u32,
    daemon: DaemonStateRecord,
}

impl Default for DaemonStateStore {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            daemon: DaemonStateRecord::new(env!("CARGO_PKG_VERSION")),
        }
    }
}

pub(super) struct ProductionState {
    daemon_path: std::path::PathBuf,
    sessions_path: std::path::PathBuf,
    _peers_path: std::path::PathBuf,
    daemon: Mutex<DaemonStateStore>,
    sessions: Mutex<SessionStore>,
    peers: Mutex<PeerStore>,
}

impl ProductionState {
    pub(super) fn load() -> Result<Self> {
        let daemon_path = config::daemon_state_path()?;
        let sessions_path = config::sessions_path()?;
        let peers_path = config::peers_path()?;
        Ok(Self {
            daemon: Mutex::new(load_store(&daemon_path)?),
            sessions: Mutex::new(load_store(&sessions_path)?),
            peers: Mutex::new(load_store(&peers_path)?),
            daemon_path,
            sessions_path,
            _peers_path: peers_path,
        })
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
    use std::{fs, net::IpAddr};

    use super::super::jobs::{JobPhase, JobState};
    use super::*;
    use crate::cli;

    fn spec() -> ProxySessionSpec {
        ProxySessionSpec {
            target: "126".to_string(),
            workspace_id: Some("Window A".to_string()),
            ssh: None,
            workspace_paths: Vec::new(),
            local_proxy: "http://127.0.0.1:10808/".to_string(),
            remote_bind: "127.0.0.1".parse::<IpAddr>().unwrap(),
            remote_port_policy: super::super::proxy_session::RemotePortPolicy {
                preferred: 17890,
                auto_pick: true,
            },
            connect_mode: cli::RouteConnectMode::ReverseLink,
            apply_policy: super::super::proxy_session::ApplyPolicy::default(),
        }
    }

    #[test]
    fn proxy_session_record_tracks_job_phase() {
        let spec = spec();
        let job = JobRecord::new(spec.job_id(), "ensure_proxy_session")
            .with_target(spec.target.clone())
            .with_workspace(spec.workspace_id.clone())
            .with_route(spec.route_id())
            .with_remote_url(Some(spec.remote_url()))
            .transition(JobState::Running, JobPhase::EnsurePeer, 35);
        let record = proxy_session_record_from_spec_and_job(&spec, &job);
        assert_eq!(record.session_id, "session:window-a");
        assert_eq!(record.state, "running");
        assert_eq!(record.phase, "ensure_peer");
        assert_eq!(record.health, "starting");
    }

    #[test]
    fn proxy_session_record_serializes_handoff_probe() {
        let spec = spec();
        let job = JobRecord::new(spec.job_id(), "ensure_proxy_session")
            .with_target(spec.target.clone())
            .with_workspace(spec.workspace_id.clone())
            .with_route(spec.route_id())
            .with_remote_url(Some(spec.remote_url()))
            .transition(JobState::WaitingRetry, JobPhase::VerifyRemotePort, 85)
            .with_retry_after_ms(250);
        let mut record = proxy_session_record_from_spec_and_job(&spec, &job);
        record.handoff_probe = Some(serde_json::to_value(HandoffProbeStatus::checking()).unwrap());

        let value = record.to_value();
        assert_eq!(value["handoff_probe"]["source"], "rust_ssh_direct_tcpip");
        assert_eq!(value["handoff_probe"]["state"], "checking");
        assert_eq!(value["handoff_probe"]["retry_after_ms"], 250);
    }

    #[test]
    fn corrupt_store_is_quarantined() {
        let dir = std::env::temp_dir().join(format!("ssh_proxy-state-test-{}", now_unix()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sessions.json");
        fs::write(&path, "{not-json").unwrap();
        let loaded: SessionStore = load_store(&path).unwrap();
        assert!(loaded.sessions.is_empty());
        assert!(!path.exists());
        assert!(fs::read_dir(&dir).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".corrupt-")
        }));
        let _ = fs::remove_dir_all(dir);
    }
}
