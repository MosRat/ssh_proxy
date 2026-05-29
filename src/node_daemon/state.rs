use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{config, repair};

use super::{
    handoff::HandoffProbeStatus,
    jobs::JobRecord,
    proxy_session::{ApplyPolicy, ProxySessionSpec, RemotePortPolicy, SshTargetSpec},
};

const STORE_VERSION: u32 = 1;

mod daemon_store;
mod file_store;
mod peer_store;
mod session_store;

use file_store::load_store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ProxySessionRecord {
    pub(super) session_id: String,
    pub(super) target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) ssh: Option<SshTargetSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) workspace_paths: Vec<String>,
    pub(super) job_id: String,
    pub(super) route_id: String,
    pub(super) local_proxy: String,
    pub(super) remote_bind: String,
    pub(super) remote_port: u16,
    pub(super) remote_url: String,
    #[serde(default)]
    pub(super) apply_policy: ApplyPolicy,
    pub(super) state: String,
    pub(super) phase: String,
    pub(super) health: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) next_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) repair_action: Option<repair::RepairAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) last_error: Option<String>,
    pub(super) remote_setup: RemoteSetupStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) handoff_probe: Option<HandoffProbeStatus>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub(super) recovery_attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) route: Option<Value>,
    pub(super) created_at_unix: u64,
    pub(super) updated_at_unix: u64,
}

impl ProxySessionRecord {
    fn from_spec_and_job(spec: &ProxySessionSpec, job: &JobRecord) -> Self {
        Self {
            session_id: spec.session_id(),
            target: spec.target.clone(),
            workspace_id: spec.workspace_id.clone(),
            ssh: spec.ssh.clone(),
            workspace_paths: spec.workspace_paths.clone(),
            job_id: job.id.clone(),
            route_id: spec.route_id(),
            local_proxy: spec.local_proxy.clone(),
            remote_bind: spec.remote_bind.to_string(),
            remote_port: spec.remote_port_policy.preferred,
            remote_url: job.remote_url.clone().unwrap_or_else(|| spec.remote_url()),
            apply_policy: spec.apply_policy.clone(),
            state: enum_value(&job.state),
            phase: enum_value(&job.phase),
            health: job_health(job),
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

    fn update_from_job(&mut self, spec: &ProxySessionSpec, job: &JobRecord) {
        self.target = spec.target.clone();
        self.workspace_id = spec.workspace_id.clone();
        self.ssh = spec.ssh.clone();
        self.workspace_paths = spec.workspace_paths.clone();
        self.route_id = spec.route_id();
        self.local_proxy = spec.local_proxy.clone();
        self.remote_bind = spec.remote_bind.to_string();
        self.remote_port = spec.remote_port_policy.preferred;
        self.remote_url = job.remote_url.clone().unwrap_or_else(|| spec.remote_url());
        self.apply_policy = spec.apply_policy.clone();
        self.state = enum_value(&job.state);
        self.phase = enum_value(&job.phase);
        self.health = job_health(job);
        self.blocker = job.blocker.clone();
        self.next_action = job.next_action.clone();
        self.repair_action = job.repair_action.clone();
        self.last_error = job.last_error.clone();
        self.updated_at_unix = job.updated_at_unix;
        if self.phase == "apply_remote_settings" {
            self.remote_setup = RemoteSetupStatus::required();
            self.remote_setup.updated_at_unix = job.updated_at_unix;
        }
        if self.phase == "healthy" && self.remote_setup.state == "pending" {
            self.remote_setup = RemoteSetupStatus::required();
            self.remote_setup.updated_at_unix = job.updated_at_unix;
        }
        self.recovery_attempts = job.recovery_attempts;
    }

    pub(super) fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({ "session_id": self.session_id }))
    }

    pub(super) fn to_spec(&self) -> Result<ProxySessionSpec> {
        Ok(ProxySessionSpec {
            target: self.target.clone(),
            workspace_id: self.workspace_id.clone(),
            ssh: self.ssh.clone(),
            workspace_paths: self.workspace_paths.clone(),
            local_proxy: self.local_proxy.clone(),
            remote_bind: self.remote_bind.parse()?,
            remote_port_policy: RemotePortPolicy {
                preferred: self.remote_port,
                auto_pick: true,
            },
            connect_mode: crate::cli::RouteConnectMode::ReverseLink,
            apply_policy: self.apply_policy.clone(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct RemoteSetupStatus {
    pub(super) state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) last_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) desired_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) applied_hash: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(super) drift_detected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) remote_url: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(super) verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) next_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) last_error: Option<String>,
    pub(super) updated_at_unix: u64,
}

impl RemoteSetupStatus {
    fn pending() -> Self {
        Self {
            state: "pending".to_string(),
            last_hash: None,
            desired_hash: None,
            applied_hash: None,
            drift_detected: false,
            remote_url: None,
            verified: false,
            next_action: Some("wait_for_proxy_session_job".to_string()),
            last_error: None,
            updated_at_unix: now_unix(),
        }
    }

    pub(super) fn required() -> Self {
        Self {
            state: "required".to_string(),
            next_action: Some("apply_remote_settings".to_string()),
            ..Self::pending()
        }
    }

    pub(super) fn running(desired_hash: Option<String>, remote_url: Option<String>) -> Self {
        Self {
            state: "running".to_string(),
            desired_hash,
            remote_url,
            next_action: Some("wait_for_remote_setup".to_string()),
            updated_at_unix: now_unix(),
            ..Self::pending()
        }
    }

    pub(super) fn applied(
        desired_hash: String,
        applied_hash: String,
        remote_url: String,
        verified: bool,
    ) -> Self {
        Self {
            state: "applied".to_string(),
            last_hash: Some(applied_hash.clone()),
            desired_hash: Some(desired_hash),
            applied_hash: Some(applied_hash),
            drift_detected: false,
            remote_url: Some(remote_url),
            verified,
            next_action: Some("monitor_remote_setup_drift".to_string()),
            last_error: None,
            updated_at_unix: now_unix(),
        }
    }

    pub(super) fn failed(
        error: String,
        desired_hash: Option<String>,
        remote_url: Option<String>,
    ) -> Self {
        Self {
            state: "failed".to_string(),
            desired_hash,
            remote_url,
            next_action: Some("rerun_apply_remote_settings".to_string()),
            last_error: Some(error),
            updated_at_unix: now_unix(),
            ..Self::pending()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PeerStatusRecord {
    pub(super) target: String,
    pub(super) state: String,
    #[serde(default = "default_unknown_health")]
    pub(super) health: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) control_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) transport: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) transport_protocols: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) service_manager: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) descriptor_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) install: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) dependency_report: Option<Value>,
    pub(super) update_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) repair_action: Option<repair::RepairAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub(super) recovery_attempts: u32,
    pub(super) updated_at_unix: u64,
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
            daemon: DaemonStateRecord::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonStateRecord {
    schema_version: u32,
    version: String,
    health: String,
    update_state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    update: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    control_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    started_at_unix: u64,
    updated_at_unix: u64,
}

impl Default for DaemonStateRecord {
    fn default() -> Self {
        let now = now_unix();
        Self {
            schema_version: STORE_VERSION,
            version: env!("CARGO_PKG_VERSION").to_string(),
            health: "starting".to_string(),
            update_state: "idle".to_string(),
            update: None,
            control_endpoint: None,
            name: None,
            started_at_unix: now,
            updated_at_unix: now,
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

fn enum_value<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

fn job_health(job: &JobRecord) -> String {
    match enum_value(&job.state).as_str() {
        "healthy" => "healthy".to_string(),
        "failed" => "failed".to_string(),
        "cancelled" => "cancelled".to_string(),
        "queued" | "running" | "waiting_retry" => "starting".to_string(),
        _ => "unknown".to_string(),
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

fn default_unknown_health() -> String {
    "unknown".to_string()
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
        let record = ProxySessionRecord::from_spec_and_job(&spec, &job);
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
        let mut record = ProxySessionRecord::from_spec_and_job(&spec, &job);
        record.handoff_probe = Some(HandoffProbeStatus::checking());

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
