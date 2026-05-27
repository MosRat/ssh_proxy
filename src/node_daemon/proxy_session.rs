use std::{net::IpAddr, sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time;

use crate::cli;

use super::{
    NodeManager, NodeRequest,
    jobs::{JobPhase, JobRecord, JobState},
    response_line,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProxySessionSpec {
    pub(crate) target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) workspace_id: Option<String>,
    pub(crate) local_proxy: String,
    pub(crate) remote_bind: IpAddr,
    pub(crate) remote_port_policy: RemotePortPolicy,
    pub(crate) connect_mode: cli::RouteConnectMode,
    pub(crate) apply_policy: ApplyPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RemotePortPolicy {
    pub(crate) preferred: u16,
    pub(crate) auto_pick: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ApplyPolicy {
    pub(crate) vscode_settings: bool,
    pub(crate) server_env: bool,
    pub(crate) git: bool,
}

impl Default for ApplyPolicy {
    fn default() -> Self {
        Self {
            vscode_settings: true,
            server_env: true,
            git: true,
        }
    }
}

impl ProxySessionSpec {
    pub(crate) fn from_up_args(args: &cli::UpArgs) -> Self {
        Self {
            target: args.target.clone(),
            workspace_id: args.workspace.clone(),
            local_proxy: args.local_proxy.clone(),
            remote_bind: args.remote_bind,
            remote_port_policy: RemotePortPolicy {
                preferred: args.remote_port,
                auto_pick: true,
            },
            connect_mode: args.connect_mode,
            apply_policy: ApplyPolicy::default(),
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

    pub(crate) fn route_id_for_key(key: &str) -> String {
        format!("v3-{}", sanitize_key(key))
    }

    pub(crate) fn job_id_for_key(key: &str) -> String {
        format!("proxy:{}", sanitize_key(key))
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

impl NodeManager {
    pub(super) async fn ensure_proxy_session(
        self: Arc<Self>,
        request: NodeRequest,
    ) -> Result<String> {
        let spec = request
            .proxy_session
            .ok_or_else(|| anyhow!("ensure_proxy_session requires proxy_session spec"))?;
        let job = JobRecord::new(spec.job_id(), "ensure_proxy_session")
            .with_target(spec.target.clone())
            .with_workspace(spec.workspace_id.clone())
            .with_route(spec.route_id())
            .with_remote_url(Some(spec.remote_url()))
            .transition(JobState::Queued, JobPhase::Queued, 0);
        let job = self.jobs.upsert(job, "proxy session accepted").await?;
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
        response_line(json!({
            "ok": true,
            "kind": "proxy_session",
            "daemon_api": "v0.3",
            "accepted": true,
            "job": job.to_value(),
            "spec": spec.to_value(),
            "route": {
                "route_id": spec.route_id(),
                "remote_url": spec.remote_url(),
                "readiness": {
                    "state": "accepted",
                    "phase": "queued",
                    "next_action": "poll_job"
                }
            },
            "remote_url": spec.remote_url(),
            "apply_remote_settings_required": true,
        }))
    }

    pub(super) async fn proxy_session_status(&self, request: NodeRequest) -> Result<String> {
        let id = request
            .id
            .or_else(|| request.proxy_session.as_ref().map(ProxySessionSpec::job_id));
        let job = match id.as_deref() {
            Some(id) => self.jobs.get(id).await,
            None => None,
        };
        response_line(json!({
            "ok": job.is_some(),
            "kind": "proxy_session_status",
            "daemon_api": "v0.3",
            "job": job.as_ref().map(JobRecord::to_value),
            "route": job.as_ref().map(route_status_from_job).unwrap_or(Value::Null),
            "remote_url": job.as_ref().and_then(|job| job.remote_url.clone()),
            "health": job.as_ref().map(|job| job_health(job)).unwrap_or("unknown"),
            "code": if job.is_some() { Value::Null } else { json!("not_found") },
        }))
    }

    pub(super) async fn proxy_session_down(&self, request: NodeRequest) -> Result<String> {
        let id = request
            .id
            .or_else(|| {
                request
                    .proxy_session
                    .as_ref()
                    .map(ProxySessionSpec::route_id)
            })
            .ok_or_else(|| anyhow!("proxy_session_down requires id or proxy_session spec"))?;
        let route_response = self.stop_route(NodeRequest::route_stop(id.clone())).await;
        let job_id = request
            .proxy_session
            .map(|spec| spec.job_id())
            .unwrap_or_else(|| format!("proxy:{id}"));
        let job = JobRecord::new(job_id, "ensure_proxy_session")
            .with_route(id.clone())
            .transition(JobState::Cancelled, JobPhase::Cancelled, 100);
        let job = self.jobs.upsert(job, "proxy session stopped").await?;
        response_line(json!({
            "ok": route_response.is_ok(),
            "kind": "proxy_session_down",
            "daemon_api": "v0.3",
            "route_id": id,
            "job": job.to_value(),
            "route_stop": route_response.ok().and_then(|text| serde_json::from_str::<Value>(&text).ok()),
        }))
    }

    async fn run_proxy_session_job(
        self: Arc<Self>,
        spec: ProxySessionSpec,
        job_id: String,
    ) -> Result<()> {
        let route_request = route_request_from_spec(&spec);
        self.jobs
            .upsert(
                job_for_phase(&spec, &job_id, JobPhase::ResolveTarget, 10),
                "resolved proxy session target",
            )
            .await?;
        self.jobs
            .upsert(
                job_for_phase(&spec, &job_id, JobPhase::EnsureLocalProxy, 20),
                "accepted local proxy egress",
            )
            .await?;
        self.jobs
            .upsert(
                job_for_phase(&spec, &job_id, JobPhase::EnsurePeer, 35),
                "ensuring remote peer through existing route planner",
            )
            .await?;
        self.jobs
            .upsert(
                job_for_phase(&spec, &job_id, JobPhase::PlanRoute, 50),
                "planned daemon-owned route",
            )
            .await?;

        let response = self.handle_route_intent(route_request).await;
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
                self.wait_for_proxy_route_ready(&spec, &job_id, remote_url)
                    .await?;
            }
            Err(err) => {
                let job = job_for_phase(&spec, &job_id, JobPhase::Failed, 100)
                    .failed(err.to_string(), Some("route_start_failed".to_string()));
                self.jobs.upsert(job, "route intent failed").await?;
            }
        }
        Ok(())
    }

    async fn wait_for_proxy_route_ready(
        &self,
        spec: &ProxySessionSpec,
        job_id: &str,
        remote_url: Option<String>,
    ) -> Result<()> {
        let route_id = spec.route_id();
        let deadline = time::Instant::now() + Duration::from_secs(12);
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
                    self.jobs.upsert(job, "route failed").await?;
                    return Ok(());
                }
                if matches!(state.as_deref(), Some("running" | "ready" | "restarting")) {
                    self.jobs
                        .upsert(
                            job_for_phase(spec, job_id, JobPhase::VerifyRemotePort, 85)
                                .with_remote_url(remote_url.clone()),
                            "route is ready for remote verification",
                        )
                        .await?;
                    self.jobs
                        .upsert(
                            job_for_phase(spec, job_id, JobPhase::ApplyRemoteSettings, 92)
                                .with_remote_url(remote_url.clone()),
                            "remote settings application required",
                        )
                        .await?;
                    self.jobs
                        .upsert(
                            job_for_phase(spec, job_id, JobPhase::Healthy, 100)
                                .transition(JobState::Healthy, JobPhase::Healthy, 100)
                                .with_remote_url(remote_url),
                            "proxy session healthy",
                        )
                        .await?;
                    return Ok(());
                }
            }
            if time::Instant::now() >= deadline {
                self.jobs
                    .upsert(
                        job_for_phase(spec, job_id, JobPhase::WaitRouteReady, 75)
                            .with_remote_url(remote_url)
                            .transition(JobState::WaitingRetry, JobPhase::WaitRouteReady, 75),
                        "route readiness still pending",
                    )
                    .await?;
                return Ok(());
            }
            time::sleep(Duration::from_millis(250)).await;
        }
    }
}

fn route_request_from_spec(spec: &ProxySessionSpec) -> NodeRequest {
    NodeRequest::route_intent(cli::RouteArgs {
        target: spec.target.clone(),
        direction: cli::RouteDirection::RemoteUsesLocal,
        connect_mode: spec.connect_mode,
        port: spec.remote_port_policy.preferred,
        bind: spec.remote_bind,
        tcp_target: None,
        endpoint: crate::control_socket::default_endpoint_string(),
        token: None,
        ssh_args: Vec::new(),
        user: None,
        ssh_port: None,
        identity: Vec::new(),
        config: None,
        known_hosts: None,
        accept_new: false,
        insecure_ignore_host_key: false,
        jump: Vec::new(),
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
        "health": job_health(job),
    })
}

fn job_health(job: &JobRecord) -> &'static str {
    match job.state {
        JobState::Healthy => "healthy",
        JobState::Failed => "failed",
        JobState::Cancelled => "cancelled",
        JobState::Queued | JobState::Running | JobState::WaitingRetry => "starting",
    }
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

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::*;

    #[test]
    fn proxy_session_spec_derives_stable_ids() {
        let spec = ProxySessionSpec {
            target: "126".to_string(),
            workspace_id: Some("Window A".to_string()),
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
}
