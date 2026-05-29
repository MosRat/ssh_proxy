use std::{net::IpAddr, time::Duration};

use anyhow::{Result, anyhow};
use serde_json::json;

use crate::{
    cli,
    node_daemon::{
        NodeManager, NodeRequest,
        handoff::{self, HandoffProbeStatus},
        jobs::{JobPhase, JobRecord, JobState},
        remote_setup, response_line,
        state::{ProxySessionRecordExt, RemoteSetupStatus},
    },
};

use super::{ApplyPolicy, ProxySessionSpec, RemotePortPolicy, error_chain, spec::sanitize_key};

impl NodeManager {
    pub(in crate::node_daemon) async fn apply_remote_settings(
        &self,
        request: NodeRequest,
    ) -> Result<String> {
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
}

pub(super) fn spec_from_apply_request(request: &NodeRequest) -> Result<ProxySessionSpec> {
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

pub(super) fn remote_endpoint_from_url(url: &str) -> Result<(IpAddr, u16)> {
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
