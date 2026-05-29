use std::{net::IpAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time;

use crate::cli;

use super::{
    NodeManager, NodeRequest,
    handoff::{self, HandoffProbeStatus},
    jobs::{JobPhase, JobRecord, JobState},
    remote_setup, response_line,
    state::RemoteSetupStatus,
};

mod state_machine;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProxySessionSpec {
    pub(crate) target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) ssh: Option<SshTargetSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) workspace_paths: Vec<String>,
    pub(crate) local_proxy: String,
    pub(crate) remote_bind: IpAddr,
    pub(crate) remote_port_policy: RemotePortPolicy,
    pub(crate) connect_mode: cli::RouteConnectMode,
    pub(crate) apply_policy: ApplyPolicy,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SshTargetSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) host_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) port: Option<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) identity: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) config: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) known_hosts: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) jump: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) accept_new: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RemotePortPolicy {
    pub(crate) preferred: u16,
    pub(crate) auto_pick: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ApplyPolicy {
    pub(crate) vscode_settings: bool,
    pub(crate) terminal_env: bool,
    pub(crate) server_env: bool,
    pub(crate) git: bool,
    pub(crate) git_global: bool,
    pub(crate) git_workspace: bool,
    pub(crate) git_force_override: bool,
    pub(crate) remote_status_file: bool,
    pub(crate) verify_remote_port: bool,
    pub(crate) no_proxy: String,
    pub(crate) proxy_support: String,
    pub(crate) server_dir: String,
}

impl Default for ApplyPolicy {
    fn default() -> Self {
        Self {
            vscode_settings: true,
            terminal_env: true,
            server_env: true,
            git: true,
            git_global: true,
            git_workspace: true,
            git_force_override: true,
            remote_status_file: true,
            verify_remote_port: true,
            no_proxy: "localhost,127.0.0.1,::1".to_string(),
            proxy_support: "override".to_string(),
            server_dir: ".vscode-server".to_string(),
        }
    }
}

impl ProxySessionSpec {
    pub(crate) fn from_up_args(args: &cli::UpArgs) -> Self {
        Self {
            target: args.target.clone(),
            workspace_id: args.workspace.clone(),
            ssh: SshTargetSpec::from_up_args(args),
            workspace_paths: args.workspace_paths.clone(),
            local_proxy: args.local_proxy.clone(),
            remote_bind: args.remote_bind,
            remote_port_policy: RemotePortPolicy {
                preferred: args.remote_port,
                auto_pick: true,
            },
            connect_mode: args.connect_mode,
            apply_policy: ApplyPolicy {
                vscode_settings: !args.no_remote_machine_settings,
                terminal_env: !args.no_terminal_env,
                server_env: !args.no_server_env,
                git: !args.no_git,
                git_global: !args.no_git_global,
                git_workspace: !args.no_git_workspace,
                git_force_override: !args.no_git_force_override,
                remote_status_file: !args.no_remote_status_file,
                verify_remote_port: !args.no_verify_remote_port,
                no_proxy: args.no_proxy.clone(),
                proxy_support: args.proxy_support.clone(),
                server_dir: args.server_dir.clone(),
            },
        }
    }

    pub(crate) fn key(&self) -> &str {
        self.workspace_id.as_deref().unwrap_or(&self.target)
    }

    pub(crate) fn route_id(&self) -> String {
        Self::route_id_for_key(self.key())
    }

    pub(crate) fn job_id(&self) -> String {
        Self::job_id_for_key(self.key())
    }

    pub(crate) fn session_id(&self) -> String {
        Self::session_id_for_key(self.key())
    }

    pub(crate) fn route_id_for_key(key: &str) -> String {
        format!("v3-{}", sanitize_key(key))
    }

    pub(crate) fn job_id_for_key(key: &str) -> String {
        format!("proxy:{}", sanitize_key(key))
    }

    pub(crate) fn session_id_for_key(key: &str) -> String {
        format!("session:{}", sanitize_key(key))
    }

    pub(crate) fn remote_url(&self) -> String {
        proxy_url_for_remote(
            &self.local_proxy,
            &self.remote_bind.to_string(),
            self.remote_port_policy.preferred,
        )
    }

    pub(crate) fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| Value::Null)
    }
}

impl SshTargetSpec {
    fn from_up_args(args: &cli::UpArgs) -> Option<Self> {
        let spec = Self {
            host_name: args.ssh_host_name.clone().filter(|value| !value.is_empty()),
            user: args.ssh_user.clone().filter(|value| !value.is_empty()),
            port: args.ssh_port,
            identity: args.ssh_identity.clone(),
            config: args.ssh_config.clone(),
            known_hosts: args.ssh_known_hosts.clone(),
            jump: args.ssh_jump.clone(),
            accept_new: args.ssh_accept_new,
        };
        (!spec.is_empty()).then_some(spec)
    }

    fn is_empty(&self) -> bool {
        self.host_name.is_none()
            && self.user.is_none()
            && self.port.is_none()
            && self.identity.is_empty()
            && self.config.is_none()
            && self.known_hosts.is_none()
            && self.jump.is_empty()
            && !self.accept_new
    }

    pub(crate) fn ssh_args(&self) -> Vec<String> {
        match self.host_name.as_deref() {
            Some(host_name) => vec!["-o".to_string(), format!("HostName={host_name}")],
            None => Vec::new(),
        }
    }
}

impl NodeManager {
    pub(super) async fn ensure_proxy_session(
        self: Arc<Self>,
        request: NodeRequest,
    ) -> Result<String> {
        let spec = request
            .proxy_session
            .ok_or_else(|| anyhow!("ensure_proxy_session requires proxy_session spec"))?;
        let job_id = spec.job_id();
        if let Some(existing) = self.jobs.get(&job_id).await {
            if state_machine::reusable_proxy_session_job(
                &existing,
                self.proxy_session_route_is_live(&spec).await,
            ) && self.proxy_session_matches_existing(&job_id, &spec).await
            {
                let session = self
                    .state
                    .upsert_session_from_job(&spec, &existing, None)
                    .await?;
                return proxy_session_accepted_response(&spec, &existing, session.to_value(), true);
            }
        }

        let job = JobRecord::new(job_id, "ensure_proxy_session")
            .with_target(spec.target.clone())
            .with_workspace(spec.workspace_id.clone())
            .with_route(spec.route_id())
            .with_remote_url(Some(spec.remote_url()))
            .transition(JobState::Queued, JobPhase::Queued, 0);
        let job = self.jobs.upsert(job, "proxy session accepted").await?;
        let session = self
            .state
            .upsert_session_from_job(&spec, &job, None)
            .await?;
        let manager = self.clone();
        let task_spec = spec.clone();
        let job_id = job.id.clone();
        tokio::spawn(async move {
            if let Err(err) = manager
                .clone()
                .run_proxy_session_job(task_spec.clone(), job_id.clone())
                .await
            {
                let failed = job_for_phase(&task_spec, &job_id, JobPhase::Failed, 100)
                    .failed(err.to_string(), Some("job_task_failed".to_string()));
                let _ = manager
                    .jobs
                    .upsert(failed, "proxy session job task failed")
                    .await;
            }
        });
        proxy_session_accepted_response(&spec, &job, session.to_value(), false)
    }

    async fn proxy_session_matches_existing(&self, job_id: &str, spec: &ProxySessionSpec) -> bool {
        let Some(session) = self.state.session_by_job(job_id).await else {
            return false;
        };
        match session.to_spec() {
            Ok(existing) => proxy_session_specs_match(&existing, spec),
            Err(_) => false,
        }
    }

    async fn proxy_session_route_is_live(&self, spec: &ProxySessionSpec) -> bool {
        let route_id = spec.route_id();
        let routes = self.routes.lock().await;
        routes
            .get(&route_id)
            .map(|task| !task.handle.is_finished())
            .unwrap_or(false)
    }

    pub(super) async fn proxy_session_status(&self, request: NodeRequest) -> Result<String> {
        let id = request
            .id
            .or_else(|| request.proxy_session.as_ref().map(ProxySessionSpec::job_id));
        let job = match id.as_deref() {
            Some(id) => self.jobs.get(id).await,
            None => None,
        };
        let session = match id.as_deref() {
            Some(id) => self.state.session_by_job(id).await,
            None => None,
        };
        let route_id = job
            .as_ref()
            .and_then(|job| job.route_id.clone())
            .or_else(|| session.as_ref().map(|session| session.route_id.clone()));
        let live_route = match route_id.as_deref() {
            Some(route_id) => self.live_route_status(route_id).await?,
            None => None,
        };
        let missing_healthy_route =
            matches!(job.as_ref().map(|job| job.state), Some(JobState::Healthy))
                && live_route.is_none()
                && route_id.is_some();
        let route = live_route
            .or_else(|| {
                if missing_healthy_route {
                    Some(missing_route_status(
                        route_id.clone(),
                        job.as_ref()
                            .and_then(|job| job.remote_url.clone())
                            .or_else(|| session.as_ref().map(|session| session.remote_url.clone())),
                    ))
                } else {
                    None
                }
            })
            .or_else(|| job.as_ref().map(route_status_from_job))
            .or_else(|| session.as_ref().and_then(|session| session.route.clone()))
            .unwrap_or(Value::Null);
        let health = (if missing_healthy_route {
            Some("starting")
        } else {
            None
        })
        .or_else(|| job.as_ref().map(state_machine::job_health))
        .or_else(|| session.as_ref().map(|session| session.health.as_str()))
        .unwrap_or("unknown");
        let ok = job.is_some() || session.is_some();
        response_line(json!({
            "ok": ok,
            "kind": "proxy_session_status",
            "daemon_api": "v0.3",
            "job": job.as_ref().map(JobRecord::to_value),
            "session": session.as_ref().map(|session| session.to_value()),
            "route": route,
            "remote_url": job.as_ref().and_then(|job| job.remote_url.clone())
                .or_else(|| session.as_ref().map(|session| session.remote_url.clone())),
            "remote_setup": session.as_ref().map(|session| serde_json::to_value(&session.remote_setup).unwrap_or(Value::Null)),
            "handoff_probe": session.as_ref().and_then(|session| session.handoff_probe.clone()),
            "health": health,
            "code": if ok { Value::Null } else { json!("not_found") },
        }))
    }

    async fn live_route_status(&self, route_id: &str) -> Result<Option<Value>> {
        let status = self.status_value().await?;
        Ok(find_route_status(&status, route_id))
    }

    pub(super) async fn proxy_session_down(&self, request: NodeRequest) -> Result<String> {
        let request_spec = request.proxy_session;
        let id = request
            .id
            .or_else(|| request_spec.as_ref().map(ProxySessionSpec::route_id))
            .ok_or_else(|| anyhow!("proxy_session_down requires id or proxy_session spec"))?;
        let cleanup_spec = match request_spec.clone() {
            Some(spec) => Some(spec),
            None => self
                .state
                .session_by_route(&id)
                .await
                .and_then(|record| record.to_spec().ok()),
        };
        let route_response = self.stop_route(NodeRequest::route_stop(id.clone())).await;
        let cleanup_response = match cleanup_spec.as_ref() {
            Some(spec) => {
                let config = {
                    let config = self.config.lock().await;
                    config.clone()
                };
                match remote_setup::cleanup_remote_settings(&config, spec, &spec.remote_url()).await
                {
                    Ok(()) => json!({
                        "ok": true,
                        "state": "cleaned",
                    }),
                    Err(err) => json!({
                        "ok": false,
                        "state": "failed",
                        "last_error": err.to_string(),
                        "next_action": "rerun_proxy_session_down"
                    }),
                }
            }
            None => json!({
                "ok": false,
                "state": "skipped",
                "last_error": "no proxy session spec was available for remote cleanup",
                "next_action": "rerun down with target/workspace while session state exists"
            }),
        };
        let job_id = request_spec
            .as_ref()
            .map(ProxySessionSpec::job_id)
            .or_else(|| cleanup_spec.as_ref().map(ProxySessionSpec::job_id))
            .unwrap_or_else(|| format!("proxy:{id}"));
        let job = JobRecord::new(job_id, "ensure_proxy_session")
            .with_route(id.clone())
            .transition(JobState::Cancelled, JobPhase::Cancelled, 100);
        let job = self.jobs.upsert(job, "proxy session stopped").await?;
        let session = self
            .state
            .cancel_session(
                &id,
                &job.id,
                route_response.as_ref().err().map(|err| err.to_string()),
            )
            .await?;
        response_line(json!({
            "ok": route_response.is_ok(),
            "kind": "proxy_session_down",
            "daemon_api": "v0.3",
            "route_id": id,
            "job": job.to_value(),
            "session": session.map(|session| session.to_value()),
            "route_stop": route_response.ok().and_then(|text| serde_json::from_str::<Value>(&text).ok()),
            "remote_cleanup": cleanup_response,
        }))
    }

    pub(super) async fn apply_remote_settings(&self, request: NodeRequest) -> Result<String> {
        let workspace = request.id.clone();
        let session = match workspace.as_deref() {
            Some(key) => {
                self.state
                    .session_by_job(&ProxySessionSpec::job_id_for_key(key))
                    .await
            }
            None => None,
        };
        let spec = match request.proxy_session.clone() {
            Some(spec) => spec,
            None => match session.as_ref() {
                Some(session) => session.to_spec()?,
                None => spec_from_apply_request(&request)?,
            },
        };
        let remote_url = request
            .remote_url
            .clone()
            .or_else(|| session.as_ref().map(|session| session.remote_url.clone()))
            .unwrap_or_else(|| spec.remote_url());
        let job_id = format!("apply-settings:{}", sanitize_key(spec.key()));
        let job = JobRecord::new(job_id.clone(), "apply_remote_settings")
            .with_target(spec.target.clone())
            .with_workspace(spec.workspace_id.clone())
            .with_route(spec.route_id())
            .with_remote_url(Some(remote_url.clone()))
            .transition(JobState::Running, JobPhase::ApplyRemoteSettings, 50)
            .with_next_action("wait_for_remote_setup");
        let job = self
            .jobs
            .upsert(job, "remote settings apply started")
            .await?;
        self.state
            .upsert_session_from_job(
                &spec,
                &job,
                session.as_ref().and_then(|session| session.route.clone()),
            )
            .await?;
        self.state
            .update_remote_setup_status(
                &spec.session_id(),
                &job_id,
                RemoteSetupStatus::running(None, Some(remote_url.clone())),
            )
            .await?;
        let config = {
            let config = self.config.lock().await;
            config.clone()
        };
        let route_value = session.as_ref().and_then(|session| session.route.clone());
        let handoff_verified = if spec.apply_policy.verify_remote_port {
            self.state
                .update_handoff_probe_status(
                    &spec.session_id(),
                    &job_id,
                    HandoffProbeStatus::checking(),
                )
                .await?;
            let job = self
                .jobs
                .upsert(
                    JobRecord::new(job_id.clone(), "apply_remote_settings")
                        .with_target(spec.target.clone())
                        .with_workspace(spec.workspace_id.clone())
                        .with_route(spec.route_id())
                        .with_remote_url(Some(remote_url.clone()))
                        .transition(JobState::WaitingRetry, JobPhase::VerifyRemotePort, 45)
                        .with_next_action("wait_for_remote_handoff")
                        .with_retry_after_ms(250),
                    "waiting for remote handoff before applying settings",
                )
                .await?;
            self.state
                .upsert_session_from_job(
                    &spec,
                    &job,
                    session.as_ref().and_then(|session| session.route.clone()),
                )
                .await?;
            match handoff::wait_remote_port_ready(&config, &spec, Duration::from_secs(90)).await {
                Ok(probe) => {
                    self.state
                        .update_handoff_probe_status(&spec.session_id(), &job_id, probe)
                        .await?;
                    true
                }
                Err(failure) => {
                    self.state
                        .update_handoff_probe_status(
                            &spec.session_id(),
                            &job_id,
                            failure.status.clone(),
                        )
                        .await?;
                    let remote_setup = RemoteSetupStatus::failed(
                        failure.message.clone(),
                        None,
                        Some(remote_url.clone()),
                    );
                    let session = self
                        .state
                        .update_remote_setup_status(
                            &spec.session_id(),
                            &job_id,
                            remote_setup.clone(),
                        )
                        .await?;
                    let job = self
                        .jobs
                        .upsert(
                            JobRecord::new(job_id, "apply_remote_settings")
                                .with_target(spec.target.clone())
                                .with_workspace(spec.workspace_id.clone())
                                .with_route(spec.route_id())
                                .with_remote_url(Some(remote_url))
                                .failed(failure.message, Some(failure.blocker))
                                .with_next_action("run ssh_proxy doctor --json"),
                            "remote handoff probe failed before applying settings",
                        )
                        .await?;
                    return response_line(json!({
                        "ok": false,
                        "kind": "vscode_apply_settings",
                        "daemon_api": "v0.3",
                        "job": job.to_value(),
                        "session": session.map(|session| session.to_value()),
                        "remote_setup": remote_setup,
                    }));
                }
            }
        } else {
            self.state
                .update_handoff_probe_status(
                    &spec.session_id(),
                    &job_id,
                    HandoffProbeStatus::skipped(),
                )
                .await?;
            false
        };
        match remote_setup::apply_remote_settings(&config, &spec, route_value.as_ref(), &remote_url)
            .await
        {
            Ok(outcome) => {
                let remote_setup = RemoteSetupStatus::applied(
                    outcome.desired_hash,
                    outcome.applied_hash,
                    outcome.remote_url,
                    handoff_verified || outcome.verified,
                );
                let session = self
                    .state
                    .update_remote_setup_status(&spec.session_id(), &job_id, remote_setup.clone())
                    .await?;
                let job = self
                    .jobs
                    .upsert(
                        JobRecord::new(job_id, "apply_remote_settings")
                            .with_target(spec.target.clone())
                            .with_workspace(spec.workspace_id.clone())
                            .with_route(spec.route_id())
                            .with_remote_url(Some(remote_url))
                            .transition(JobState::Healthy, JobPhase::Healthy, 100)
                            .with_next_action("monitor_remote_setup_drift"),
                        "remote settings apply healthy",
                    )
                    .await?;
                response_line(json!({
                    "ok": true,
                    "kind": "vscode_apply_settings",
                    "daemon_api": "v0.3",
                    "job": job.to_value(),
                    "session": session.map(|session| session.to_value()),
                    "remote_setup": remote_setup,
                }))
            }
            Err(err) => {
                let error = error_chain(&err);
                let remote_setup =
                    RemoteSetupStatus::failed(error.clone(), None, Some(remote_url.clone()));
                let session = self
                    .state
                    .update_remote_setup_status(&spec.session_id(), &job_id, remote_setup.clone())
                    .await?;
                let job = self
                    .jobs
                    .upsert(
                        JobRecord::new(job_id, "apply_remote_settings")
                            .with_target(spec.target.clone())
                            .with_workspace(spec.workspace_id.clone())
                            .with_route(spec.route_id())
                            .with_remote_url(Some(remote_url))
                            .failed(error, Some("remote_setup_failed".to_string()))
                            .with_next_action("rerun_vscode_apply_settings"),
                        "remote settings apply failed",
                    )
                    .await?;
                response_line(json!({
                    "ok": false,
                    "kind": "vscode_apply_settings",
                    "daemon_api": "v0.3",
                    "job": job.to_value(),
                    "session": session.map(|session| session.to_value()),
                    "remote_setup": remote_setup,
                }))
            }
        }
    }

    pub(super) async fn reconcile_proxy_sessions(&self) -> Result<()> {
        for session in self.state.unfinished_sessions().await {
            let job = JobRecord::new(session.job_id.clone(), "ensure_proxy_session")
                .with_target(session.target.clone())
                .with_workspace(session.workspace_id.clone())
                .with_route(session.route_id.clone())
                .with_remote_url(Some(session.remote_url.clone()))
                .transition(JobState::WaitingRetry, JobPhase::Reconciling, 5)
                .with_next_action("rerun_ensure_proxy_session");
            self.jobs
                .upsert(
                    job,
                    "proxy session requires reconciliation after daemon restart",
                )
                .await?;
        }
        Ok(())
    }

    async fn run_proxy_session_job(
        self: Arc<Self>,
        spec: ProxySessionSpec,
        job_id: String,
    ) -> Result<()> {
        let route_request = route_request_from_spec(&spec);
        let step = state_machine::resolve_target_step();
        self.proxy_job_phase(&spec, &job_id, step.phase, step.progress, step.message)
            .await?;
        if let Err(err) = validate_proxy_session_spec(&spec) {
            let job = job_for_phase(&spec, &job_id, JobPhase::Failed, 100)
                .failed(err.to_string(), Some("invalid_local_proxy".to_string()))
                .with_next_action("set remoteProxy.localProxy.url to http:// or socks5h://");
            let job = self
                .jobs
                .upsert(job, "proxy session validation failed")
                .await?;
            self.state
                .upsert_session_from_job(&spec, &job, None)
                .await?;
            return Ok(());
        }
        for step in [
            state_machine::validate_local_proxy_step(),
            state_machine::select_remote_port_step(),
            state_machine::ensure_peer_step(),
        ] {
            self.proxy_job_phase(&spec, &job_id, step.phase, step.progress, step.message)
                .await?;
        }
        if !self
            .ensure_remote_peer_for_proxy_session(&spec, &job_id)
            .await?
        {
            return Ok(());
        }
        for step in [
            state_machine::ensure_transport_step(),
            state_machine::plan_route_step(),
        ] {
            self.proxy_job_phase(&spec, &job_id, step.phase, step.progress, step.message)
                .await?;
        }

        let response = self.handle_route_intent(route_request.clone()).await;
        match response {
            Ok(response) => {
                let parsed = serde_json::from_str::<Value>(&response).unwrap_or(Value::Null);
                let remote_url = parsed
                    .get("remote_url")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| Some(spec.remote_url()));
                self.jobs
                    .upsert(
                        job_for_phase(&spec, &job_id, JobPhase::StartRoute, 70)
                            .with_remote_url(remote_url.clone()),
                        "route intent accepted",
                    )
                    .await?;
                if let Some(job) = self.jobs.get(&job_id).await {
                    self.state
                        .upsert_session_from_job(&spec, &job, None)
                        .await?;
                }
                self.wait_for_proxy_route_ready(&spec, &job_id, remote_url)
                    .await?;
            }
            Err(err) => {
                let error = err.to_string();
                if state_machine::route_start_conflict_is_repairable(&error) {
                    let job = self
                        .jobs
                        .upsert(
                            job_for_phase(&spec, &job_id, JobPhase::StartRoute, 68)
                                .transition(JobState::WaitingRetry, JobPhase::StartRoute, 68)
                                .with_next_action("restart_conflicting_route")
                                .with_retry_after_ms(250)
                                .with_recovery_attempts(1),
                            "route conflict detected; restarting daemon-owned route",
                        )
                        .await?;
                    self.state
                        .upsert_session_from_job(&spec, &job, None)
                        .await?;
                    let _ = self
                        .stop_route(NodeRequest::route_stop(spec.route_id()))
                        .await;
                    time::sleep(Duration::from_millis(250)).await;
                    match self.handle_route_intent(route_request).await {
                        Ok(response) => {
                            let parsed =
                                serde_json::from_str::<Value>(&response).unwrap_or(Value::Null);
                            let remote_url = parsed
                                .get("remote_url")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                                .or_else(|| Some(spec.remote_url()));
                            self.jobs
                                .upsert(
                                    job_for_phase(&spec, &job_id, JobPhase::StartRoute, 70)
                                        .with_remote_url(remote_url.clone())
                                        .with_recovery_attempts(1),
                                    "route conflict repaired and route intent accepted",
                                )
                                .await?;
                            if let Some(job) = self.jobs.get(&job_id).await {
                                self.state
                                    .upsert_session_from_job(&spec, &job, None)
                                    .await?;
                            }
                            self.wait_for_proxy_route_ready(&spec, &job_id, remote_url)
                                .await?;
                        }
                        Err(retry_err) => {
                            let job = job_for_phase(&spec, &job_id, JobPhase::Failed, 100)
                                .with_recovery_attempts(1)
                                .failed(
                                    retry_err.to_string(),
                                    Some("route_already_running_different_spec".to_string()),
                                )
                                .with_next_action("ssh_proxy down --target <target> --json");
                            let job = self
                                .jobs
                                .upsert(job, "route conflict repair failed")
                                .await?;
                            self.state
                                .upsert_session_from_job(&spec, &job, None)
                                .await?;
                        }
                    }
                } else {
                    let blocker = state_machine::route_start_blocker(&error);
                    let job = job_for_phase(&spec, &job_id, JobPhase::Failed, 100)
                        .failed(error, Some(blocker));
                    let job = self.jobs.upsert(job, "route intent failed").await?;
                    self.state
                        .upsert_session_from_job(&spec, &job, None)
                        .await?;
                }
            }
        }
        Ok(())
    }

    async fn proxy_job_phase(
        &self,
        spec: &ProxySessionSpec,
        job_id: &str,
        phase: JobPhase,
        progress: u8,
        message: &str,
    ) -> Result<JobRecord> {
        let job = self
            .jobs
            .upsert(job_for_phase(spec, job_id, phase, progress), message)
            .await?;
        self.state.upsert_session_from_job(spec, &job, None).await?;
        Ok(job)
    }

    async fn wait_for_proxy_route_ready(
        &self,
        spec: &ProxySessionSpec,
        job_id: &str,
        remote_url: Option<String>,
    ) -> Result<()> {
        let route_id = spec.route_id();
        let deadline = time::Instant::now() + Duration::from_secs(90);
        loop {
            let status = self.status_value().await?;
            if let Some(route) = find_route_status(&status, &route_id) {
                let state = route_state(&route);
                if matches!(state.as_deref(), Some("error" | "failed")) {
                    let error = route
                        .get("last_error")
                        .and_then(Value::as_str)
                        .unwrap_or("route failed")
                        .to_string();
                    let job = job_for_phase(spec, job_id, JobPhase::Failed, 100)
                        .with_remote_url(remote_url)
                        .failed(error, Some("route_failed".to_string()));
                    let job = self.jobs.upsert(job, "route failed").await?;
                    self.state
                        .upsert_session_from_job(spec, &job, Some(route))
                        .await?;
                    return Ok(());
                }
                if matches!(state.as_deref(), Some("running" | "ready" | "restarting")) {
                    let remote_url_value = remote_url.clone().unwrap_or_else(|| spec.remote_url());
                    let job = self
                        .jobs
                        .upsert(
                            job_for_phase(spec, job_id, JobPhase::VerifyRemotePort, 85)
                                .with_remote_url(remote_url.clone()),
                            "route is ready for remote verification",
                        )
                        .await?;
                    self.state
                        .upsert_session_from_job(spec, &job, Some(route.clone()))
                        .await?;
                    let handoff_verified = if spec.apply_policy.verify_remote_port {
                        let checking = HandoffProbeStatus::checking();
                        self.state
                            .update_handoff_probe_status(&spec.session_id(), job_id, checking)
                            .await?;
                        let job = self
                            .jobs
                            .upsert(
                                job_for_phase(spec, job_id, JobPhase::VerifyRemotePort, 85)
                                    .with_remote_url(remote_url.clone())
                                    .transition(
                                        JobState::WaitingRetry,
                                        JobPhase::VerifyRemotePort,
                                        85,
                                    )
                                    .with_next_action("wait_for_remote_handoff")
                                    .with_retry_after_ms(250),
                                "waiting for remote handoff probe",
                            )
                            .await?;
                        self.state
                            .upsert_session_from_job(spec, &job, Some(route.clone()))
                            .await?;
                        let config = {
                            let config = self.config.lock().await;
                            config.clone()
                        };
                        match handoff::wait_remote_port_ready(
                            &config,
                            spec,
                            Duration::from_secs(90),
                        )
                        .await
                        {
                            Ok(probe) => {
                                self.state
                                    .update_handoff_probe_status(&spec.session_id(), job_id, probe)
                                    .await?;
                                true
                            }
                            Err(failure) => {
                                self.state
                                    .update_handoff_probe_status(
                                        &spec.session_id(),
                                        job_id,
                                        failure.status.clone(),
                                    )
                                    .await?;
                                let job = job_for_phase(spec, job_id, JobPhase::Failed, 100)
                                    .with_remote_url(Some(remote_url_value))
                                    .failed(failure.message, Some(failure.blocker))
                                    .with_next_action("run ssh_proxy doctor --json");
                                let job =
                                    self.jobs.upsert(job, "remote handoff probe failed").await?;
                                self.state
                                    .upsert_session_from_job(spec, &job, Some(route.clone()))
                                    .await?;
                                return Ok(());
                            }
                        }
                    } else {
                        self.state
                            .update_handoff_probe_status(
                                &spec.session_id(),
                                job_id,
                                HandoffProbeStatus::skipped(),
                            )
                            .await?;
                        false
                    };
                    let job = self
                        .jobs
                        .upsert(
                            job_for_phase(spec, job_id, JobPhase::ApplyRemoteSettings, 92)
                                .with_remote_url(remote_url.clone()),
                            "remote settings application required",
                        )
                        .await?;
                    self.state
                        .upsert_session_from_job(spec, &job, Some(route.clone()))
                        .await?;
                    self.state
                        .update_remote_setup_status(
                            &spec.session_id(),
                            job_id,
                            RemoteSetupStatus::running(None, Some(remote_url_value.clone())),
                        )
                        .await?;
                    let route_for_setup = route.clone();
                    let config = {
                        let config = self.config.lock().await;
                        config.clone()
                    };
                    match remote_setup::apply_remote_settings(
                        &config,
                        spec,
                        Some(&route_for_setup),
                        &remote_url_value,
                    )
                    .await
                    {
                        Ok(outcome) => {
                            self.state
                                .update_remote_setup_status(
                                    &spec.session_id(),
                                    job_id,
                                    RemoteSetupStatus::applied(
                                        outcome.desired_hash,
                                        outcome.applied_hash,
                                        outcome.remote_url,
                                        handoff_verified || outcome.verified,
                                    ),
                                )
                                .await?;
                        }
                        Err(err) => {
                            let error = error_chain(&err);
                            let job = job_for_phase(spec, job_id, JobPhase::Failed, 100)
                                .with_remote_url(Some(remote_url_value.clone()))
                                .failed(error.clone(), Some("remote_setup_failed".to_string()))
                                .with_next_action("rerun_apply_remote_settings");
                            let job = self.jobs.upsert(job, "remote settings failed").await?;
                            self.state
                                .upsert_session_from_job(spec, &job, Some(route.clone()))
                                .await?;
                            self.state
                                .update_remote_setup_status(
                                    &spec.session_id(),
                                    job_id,
                                    RemoteSetupStatus::failed(error, None, Some(remote_url_value)),
                                )
                                .await?;
                            return Ok(());
                        }
                    }
                    let job = self
                        .jobs
                        .upsert(
                            job_for_phase(spec, job_id, JobPhase::HealthMonitoring, 98)
                                .with_remote_url(Some(remote_url_value.clone())),
                            "proxy session entered health monitoring",
                        )
                        .await?;
                    self.state
                        .upsert_session_from_job(spec, &job, Some(route.clone()))
                        .await?;
                    let job = self
                        .jobs
                        .upsert(
                            job_for_phase(spec, job_id, JobPhase::Healthy, 100)
                                .transition(JobState::Healthy, JobPhase::Healthy, 100)
                                .with_remote_url(Some(remote_url_value)),
                            "proxy session healthy",
                        )
                        .await?;
                    self.state
                        .upsert_session_from_job(spec, &job, Some(route))
                        .await?;
                    return Ok(());
                }
            }
            if time::Instant::now() >= deadline {
                let job = job_for_phase(spec, job_id, JobPhase::Failed, 100)
                    .with_remote_url(remote_url)
                    .failed(
                        "route readiness timed out before remote handoff could start",
                        Some("route_ready_timeout".to_string()),
                    )
                    .with_next_action("rerun_ensure_proxy_session");
                let job = self.jobs.upsert(job, "route readiness timed out").await?;
                self.state.upsert_session_from_job(spec, &job, None).await?;
                return Ok(());
            }
            time::sleep(Duration::from_millis(250)).await;
        }
    }
}

fn route_request_from_spec(spec: &ProxySessionSpec) -> NodeRequest {
    let ssh = spec.ssh.as_ref();
    NodeRequest::route_intent(cli::RouteArgs {
        target: spec.target.clone(),
        direction: cli::RouteDirection::RemoteUsesLocal,
        connect_mode: spec.connect_mode,
        port: spec.remote_port_policy.preferred,
        bind: spec.remote_bind,
        tcp_target: None,
        endpoint: crate::control_socket::default_endpoint_string(),
        token: None,
        ssh_args: ssh.map(SshTargetSpec::ssh_args).unwrap_or_default(),
        user: ssh.and_then(|ssh| ssh.user.clone()),
        ssh_port: ssh.and_then(|ssh| ssh.port),
        identity: ssh.map(|ssh| ssh.identity.clone()).unwrap_or_default(),
        config: ssh.and_then(|ssh| ssh.config.clone()),
        known_hosts: ssh.and_then(|ssh| ssh.known_hosts.clone()),
        accept_new: ssh.is_some_and(|ssh| ssh.accept_new),
        insecure_ignore_host_key: false,
        jump: ssh.map(|ssh| ssh.jump.clone()).unwrap_or_default(),
        remote_path: None,
        remote_bin: None,
        deploy: cli::DeployMode::Auto,
        remote_os: cli::RemoteOs::Auto,
        remote_transport: cli::RemoteTransport::Auto,
        remote_tcp: None,
        remote_control: None,
        remote_quic: None,
        remote_tls: None,
        remote_ca: None,
        remote_name: "localhost".to_string(),
        remote_token: None,
        egress_proxy: Some(spec.local_proxy.clone()),
        reconnect_delay_secs: None,
        reconnect_max_delay_secs: None,
        connect_timeout_secs: None,
        quic_max_bidi_streams: None,
        quic_stream_receive_window: None,
        quic_receive_window: None,
        quic_keep_alive_interval_secs: None,
        quic_idle_timeout_secs: None,
        transport_pool_size: None,
        workload_hint: Some(cli::RouteWorkloadHint::Large),
        ssh_session_pool_size: None,
        no_reconnect: false,
        local_peer: None,
        allow_plain_tcp: false,
        id: Some(spec.route_id()),
        volatile: true,
        dry_run: false,
        explain: false,
        json: true,
    })
}

fn job_for_phase(
    spec: &ProxySessionSpec,
    job_id: &str,
    phase: JobPhase,
    progress: u8,
) -> JobRecord {
    JobRecord::new(job_id.to_string(), "ensure_proxy_session")
        .with_target(spec.target.clone())
        .with_workspace(spec.workspace_id.clone())
        .with_route(spec.route_id())
        .with_remote_url(Some(spec.remote_url()))
        .transition(JobState::Running, phase, progress)
}

fn proxy_session_accepted_response(
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

fn proxy_session_specs_match(left: &ProxySessionSpec, right: &ProxySessionSpec) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    normalize_proxy_session_spec_for_live_reuse(&mut left);
    normalize_proxy_session_spec_for_live_reuse(&mut right);
    serde_json::to_value(left).ok() == serde_json::to_value(right).ok()
}

fn normalize_proxy_session_spec_for_live_reuse(spec: &mut ProxySessionSpec) {
    if let Some(ssh) = spec.ssh.as_mut() {
        ssh.identity.clear();
    }
}

fn find_route_status(status: &Value, route_id: &str) -> Option<Value> {
    status
        .get("routes")
        .and_then(Value::as_array)?
        .iter()
        .find(|route| route.get("id").and_then(Value::as_str) == Some(route_id))
        .cloned()
}

fn route_state(route: &Value) -> Option<String> {
    route
        .pointer("/readiness/state")
        .and_then(Value::as_str)
        .or_else(|| route.get("state").and_then(Value::as_str))
        .map(str::to_string)
}

fn route_status_from_job(job: &JobRecord) -> Value {
    json!({
        "route_id": job.route_id,
        "remote_url": job.remote_url,
            "health": state_machine::job_health(job),
    })
}

fn missing_route_status(route_id: Option<String>, remote_url: Option<String>) -> Value {
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

fn error_chain(err: &anyhow::Error) -> String {
    format!("{err:#}")
}

fn proxy_url_for_remote(local_proxy: &str, remote_bind: &str, remote_port: u16) -> String {
    let Some((scheme, rest)) = local_proxy.split_once("://") else {
        return format!("http://{remote_bind}:{remote_port}");
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let suffix = &rest[authority_end..];
    let userinfo = authority
        .rsplit_once('@')
        .map(|(userinfo, _)| format!("{userinfo}@"))
        .unwrap_or_default();
    format!("{scheme}://{userinfo}{remote_bind}:{remote_port}{suffix}")
}

fn validate_proxy_session_spec(spec: &ProxySessionSpec) -> Result<()> {
    let (scheme, rest) = spec
        .local_proxy
        .split_once("://")
        .ok_or_else(|| anyhow!("local proxy URL must include a scheme"))?;
    match scheme {
        "http" | "socks5" | "socks5h" => {}
        _ => {
            return Err(anyhow!(
                "unsupported local proxy scheme {scheme:?}; use http:// or socks5h://"
            ));
        }
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let authority = authority
        .rsplit_once('@')
        .map(|(_, endpoint)| endpoint)
        .unwrap_or(authority);
    if authority.is_empty() {
        return Err(anyhow!("local proxy URL is missing a host"));
    }
    if let Some(port) = authority
        .strip_prefix('[')
        .and_then(|rest| rest.find(']').map(|end| &rest[end + 1..]))
        .and_then(|tail| tail.strip_prefix(':'))
        .or_else(|| {
            authority
                .rsplit_once(':')
                .filter(|(host, _)| !host.contains(':'))
                .map(|(_, port)| port)
        })
    {
        port.parse::<u16>()
            .map_err(|_| anyhow!("local proxy URL has an invalid port"))?;
    }
    Ok(())
}

fn spec_from_apply_request(request: &NodeRequest) -> Result<ProxySessionSpec> {
    let target = request
        .alias
        .clone()
        .ok_or_else(|| anyhow!("apply_remote_settings requires target"))?;
    let workspace = request
        .id
        .clone()
        .ok_or_else(|| anyhow!("apply_remote_settings requires workspace"))?;
    let remote_url = request
        .remote_url
        .clone()
        .ok_or_else(|| anyhow!("apply_remote_settings requires remote_url"))?;
    let (remote_bind, remote_port) = remote_endpoint_from_url(&remote_url)?;
    Ok(ProxySessionSpec {
        target,
        workspace_id: Some(workspace),
        ssh: None,
        workspace_paths: Vec::new(),
        local_proxy: remote_url,
        remote_bind,
        remote_port_policy: RemotePortPolicy {
            preferred: remote_port,
            auto_pick: true,
        },
        connect_mode: cli::RouteConnectMode::ReverseLink,
        apply_policy: ApplyPolicy::default(),
    })
}

fn remote_endpoint_from_url(url: &str) -> Result<(IpAddr, u16)> {
    let (_, rest) = url
        .split_once("://")
        .ok_or_else(|| anyhow!("remote proxy URL must include a scheme"))?;
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let authority = authority
        .rsplit_once('@')
        .map(|(_, endpoint)| endpoint)
        .unwrap_or(authority);
    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        let (host, tail) = stripped
            .split_once(']')
            .ok_or_else(|| anyhow!("remote proxy URL has an invalid IPv6 host"))?;
        let port = tail
            .strip_prefix(':')
            .ok_or_else(|| anyhow!("remote proxy URL is missing a port"))?;
        (host, port)
    } else {
        authority
            .rsplit_once(':')
            .ok_or_else(|| anyhow!("remote proxy URL is missing a port"))?
    };
    let bind = host
        .parse::<IpAddr>()
        .map_err(|_| anyhow!("remote proxy URL host must be an IP address"))?;
    let port = port
        .parse::<u16>()
        .map_err(|_| anyhow!("remote proxy URL port is invalid"))?;
    Ok((bind, port))
}

fn sanitize_key(key: &str) -> String {
    let normalized = key
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let trimmed = normalized
        .trim_matches('-')
        .chars()
        .take(64)
        .collect::<String>();
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use std::{net::IpAddr, path::PathBuf};

    use super::*;

    #[test]
    fn proxy_session_spec_derives_stable_ids() {
        let spec = ProxySessionSpec {
            target: "126".to_string(),
            workspace_id: Some("Window A".to_string()),
            ssh: None,
            workspace_paths: Vec::new(),
            local_proxy: "http://127.0.0.1:10808/".to_string(),
            remote_bind: "127.0.0.1".parse::<IpAddr>().unwrap(),
            remote_port_policy: RemotePortPolicy {
                preferred: 17890,
                auto_pick: true,
            },
            connect_mode: cli::RouteConnectMode::ReverseLink,
            apply_policy: ApplyPolicy::default(),
        };
        assert_eq!(spec.route_id(), "v3-window-a");
        assert_eq!(spec.job_id(), "proxy:window-a");
        assert_eq!(spec.remote_url(), "http://127.0.0.1:17890/");
    }

    #[test]
    fn proxy_url_preserves_userinfo_and_suffix() {
        assert_eq!(
            proxy_url_for_remote("http://user:pass@127.0.0.1:10808/path", "127.0.0.1", 17890),
            "http://user:pass@127.0.0.1:17890/path",
        );
    }

    #[test]
    fn healthy_proxy_session_job_requires_live_route_for_reuse() {
        let healthy = JobRecord::new("proxy:window-a", "ensure_proxy_session").transition(
            JobState::Healthy,
            JobPhase::Healthy,
            100,
        );
        let running = JobRecord::new("proxy:window-a", "ensure_proxy_session").transition(
            JobState::Running,
            JobPhase::EnsurePeer,
            35,
        );

        assert!(!state_machine::reusable_proxy_session_job(&healthy, false));
        assert!(state_machine::reusable_proxy_session_job(&healthy, true));
        assert!(state_machine::reusable_proxy_session_job(&running, false));
    }

    #[test]
    fn route_request_uses_client_supplied_ssh_target() {
        let spec = ProxySessionSpec {
            target: "102".to_string(),
            workspace_id: Some("Window A".to_string()),
            ssh: Some(SshTargetSpec {
                host_name: Some("10.10.100.71".to_string()),
                user: Some("wenhongli".to_string()),
                port: Some(10022),
                identity: vec![PathBuf::from("C:/Users/whl/.ssh/id_rsa")],
                config: Some(PathBuf::from("C:/Users/whl/.ssh/config")),
                known_hosts: Some(PathBuf::from("C:/Users/whl/.ssh/known_hosts")),
                jump: vec!["hub".to_string()],
                accept_new: true,
            }),
            workspace_paths: Vec::new(),
            local_proxy: "http://127.0.0.1:10808/".to_string(),
            remote_bind: "127.0.0.1".parse::<IpAddr>().unwrap(),
            remote_port_policy: RemotePortPolicy {
                preferred: 17890,
                auto_pick: true,
            },
            connect_mode: cli::RouteConnectMode::ReverseLink,
            apply_policy: ApplyPolicy::default(),
        };

        let request = route_request_from_spec(&spec);
        let route = request.route.expect("route args");

        assert_eq!(route.target, "102");
        assert_eq!(route.user.as_deref(), Some("wenhongli"));
        assert_eq!(route.ssh_port, Some(10022));
        assert_eq!(
            route.identity,
            vec![PathBuf::from("C:/Users/whl/.ssh/id_rsa")]
        );
        assert_eq!(
            route.config,
            Some(PathBuf::from("C:/Users/whl/.ssh/config"))
        );
        assert_eq!(
            route.known_hosts,
            Some(PathBuf::from("C:/Users/whl/.ssh/known_hosts"))
        );
        assert_eq!(route.jump, vec!["hub"]);
        assert!(route.accept_new);
        assert_eq!(route.ssh_args, vec!["-o", "HostName=10.10.100.71"]);
    }

    #[test]
    fn proxy_session_reuse_ignores_identity_enrichment() {
        let mut existing = ProxySessionSpec {
            target: "125".to_string(),
            workspace_id: Some("wenhongli@172.18.116.125".to_string()),
            ssh: Some(SshTargetSpec {
                host_name: Some("172.18.116.125".to_string()),
                user: Some("wenhongli".to_string()),
                port: None,
                identity: Vec::new(),
                config: Some(PathBuf::from("C:/Users/whl/.ssh/config")),
                known_hosts: Some(PathBuf::from("C:/Users/whl/.ssh/known_hosts")),
                jump: Vec::new(),
                accept_new: true,
            }),
            workspace_paths: Vec::new(),
            local_proxy: "http://127.0.0.1:10808/".to_string(),
            remote_bind: "127.0.0.1".parse::<IpAddr>().unwrap(),
            remote_port_policy: RemotePortPolicy {
                preferred: 17890,
                auto_pick: true,
            },
            connect_mode: cli::RouteConnectMode::ReverseLink,
            apply_policy: ApplyPolicy::default(),
        };
        let mut enriched = existing.clone();
        enriched.ssh.as_mut().unwrap().identity = vec![
            PathBuf::from("C:/Users/whl/.ssh/id_rsa"),
            PathBuf::from("C:/Users/whl/.ssh/id_ed25519"),
        ];

        assert!(proxy_session_specs_match(&existing, &enriched));

        existing.remote_port_policy.preferred = 17891;
        assert!(!proxy_session_specs_match(&existing, &enriched));
    }

    #[test]
    fn validates_supported_local_proxy_urls() {
        let mut spec = ProxySessionSpec {
            target: "126".to_string(),
            workspace_id: None,
            ssh: None,
            workspace_paths: Vec::new(),
            local_proxy: "socks5h://user:pass@[::1]:1080/path".to_string(),
            remote_bind: "127.0.0.1".parse::<IpAddr>().unwrap(),
            remote_port_policy: RemotePortPolicy {
                preferred: 17890,
                auto_pick: true,
            },
            connect_mode: cli::RouteConnectMode::ReverseLink,
            apply_policy: ApplyPolicy::default(),
        };
        validate_proxy_session_spec(&spec).unwrap();

        spec.local_proxy = "https://127.0.0.1:10808/".to_string();
        assert!(validate_proxy_session_spec(&spec).is_err());

        spec.local_proxy = "http://127.0.0.1:abc/".to_string();
        assert!(validate_proxy_session_spec(&spec).is_err());
    }

    #[test]
    fn apply_settings_request_builds_spec_from_remote_url() {
        let request = NodeRequest::apply_remote_settings(
            "126".to_string(),
            "Window A".to_string(),
            "http://127.0.0.1:17890/".to_string(),
        );

        let spec = spec_from_apply_request(&request).unwrap();

        assert_eq!(spec.target, "126");
        assert_eq!(spec.workspace_id.as_deref(), Some("Window A"));
        assert_eq!(spec.remote_bind, "127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(spec.remote_port_policy.preferred, 17890);
    }

    #[test]
    fn remote_endpoint_parser_accepts_ipv6_urls() {
        let (host, port) = remote_endpoint_from_url("http://[::1]:17890/").unwrap();

        assert_eq!(host, "::1".parse::<IpAddr>().unwrap());
        assert_eq!(port, 17890);
    }
}
