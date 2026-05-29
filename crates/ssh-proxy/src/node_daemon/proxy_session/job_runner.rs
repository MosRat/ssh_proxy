use std::{sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use serde_json::Value;
use tokio::time;

use crate::cli;
use crate::node_daemon::{
    NodeManager, NodeRequest,
    jobs::{JobPhase, JobRecord, JobState},
};

use super::{ProxySessionSpec, SshTargetSpec, state_machine};

impl NodeManager {
    pub(super) async fn run_proxy_session_job(
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
}

pub(in crate::node_daemon::proxy_session) fn route_request_from_spec(
    spec: &ProxySessionSpec,
) -> NodeRequest {
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

pub(in crate::node_daemon::proxy_session) fn job_for_phase(
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

pub(in crate::node_daemon::proxy_session) fn validate_proxy_session_spec(
    spec: &ProxySessionSpec,
) -> Result<()> {
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
