use std::{sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use serde_json::Value;
use tokio::time;
use tracing::{info, warn};

use crate::cli;
use crate::node_daemon::{
    NodeManager, NodeRequest,
    jobs::{JobPhase, JobRecord, JobState},
};

use super::{ProxySessionSpec, SshTargetSpec, route_ready::RouteReadyOutcome, state_machine};

impl NodeManager {
    pub(super) async fn run_proxy_session_job(
        self: Arc<Self>,
        spec: ProxySessionSpec,
        job_id: String,
    ) -> Result<()> {
        let session_id = spec.session_id();
        let route_id = spec.route_id();
        info!(
            job_id = %job_id,
            session_id = %session_id,
            route_id = %route_id,
            peer = %spec.target,
            "proxy session job started"
        );
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

        let candidates = spec.remote_port_policy.candidates();
        let candidate_count = candidates.len();
        let mut last_retryable_failure: Option<PortAttemptFailure> = None;
        for (attempt_index, port) in candidates.iter().copied().enumerate() {
            let attempt_spec = spec.with_remote_port(port);
            if attempt_index > 0 {
                let job = self
                    .jobs
                    .upsert(
                        job_for_phase(&attempt_spec, &job_id, JobPhase::SelectRemotePort, 24)
                            .transition(JobState::WaitingRetry, JobPhase::SelectRemotePort, 24)
                            .with_next_action("try_next_remote_port")
                            .with_retry_after_ms(250)
                            .with_recovery_attempts(attempt_index as u32),
                        "remote port conflict detected; trying next candidate",
                    )
                    .await?;
                self.state
                    .upsert_session_from_job(&attempt_spec, &job, None)
                    .await?;
            }
            match self
                .run_proxy_session_route_attempt(&attempt_spec, &job_id, attempt_index as u32)
                .await?
            {
                RouteReadyOutcome::Healthy => return Ok(()),
                RouteReadyOutcome::Failed {
                    blocker,
                    message,
                    remote_url,
                } => {
                    if state_machine::remote_port_failure_is_retryable(&blocker, &message)
                        && attempt_index + 1 < candidate_count
                        && spec.remote_port_policy.auto_pick
                    {
                        warn!(
                            job_id = %job_id,
                            session_id = %session_id,
                            route_id = %route_id,
                            peer = %spec.target,
                            port,
                            blocker = %blocker,
                            error = %message,
                            "remote port candidate failed; trying next port"
                        );
                        let _ = self
                            .stop_route(NodeRequest::route_stop(attempt_spec.route_id()))
                            .await;
                        time::sleep(Duration::from_millis(250)).await;
                        last_retryable_failure = Some(PortAttemptFailure {
                            port,
                            blocker,
                            message,
                            remote_url,
                        });
                        continue;
                    }
                    if state_machine::remote_port_failure_is_retryable(&blocker, &message)
                        && spec.remote_port_policy.auto_pick
                    {
                        last_retryable_failure = Some(PortAttemptFailure {
                            port,
                            blocker,
                            message,
                            remote_url,
                        });
                        break;
                    }
                    return Ok(());
                }
            }
        }
        if let Some(failure) = last_retryable_failure {
            self.record_remote_port_range_exhausted(&spec, &job_id, failure, candidate_count)
                .await?;
        }
        Ok(())
    }

    async fn run_proxy_session_route_attempt(
        &self,
        spec: &ProxySessionSpec,
        job_id: &str,
        recovery_attempts: u32,
    ) -> Result<RouteReadyOutcome> {
        let route_request = route_request_from_spec(spec);
        let response = self.handle_route_intent(route_request.clone()).await;
        match response {
            Ok(response) => {
                self.record_route_intent_accepted(spec, job_id, &response, recovery_attempts)
                    .await
            }
            Err(err) => {
                let error = err.to_string();
                if state_machine::route_start_conflict_is_repairable(&error) {
                    warn!(
                        job_id = %job_id,
                        session_id = %spec.session_id(),
                        route_id = %spec.route_id(),
                        peer = %spec.target,
                        port = spec.remote_port_policy.preferred,
                        error = %error,
                        "proxy session route conflict detected; restarting daemon-owned route"
                    );
                    let job = self
                        .jobs
                        .upsert(
                            job_for_phase(spec, job_id, JobPhase::StartRoute, 68)
                                .transition(JobState::WaitingRetry, JobPhase::StartRoute, 68)
                                .with_next_action("restart_conflicting_route")
                                .with_retry_after_ms(250)
                                .with_recovery_attempts(recovery_attempts + 1),
                            "route conflict detected; restarting daemon-owned route",
                        )
                        .await?;
                    self.state.upsert_session_from_job(spec, &job, None).await?;
                    let _ = self
                        .stop_route(NodeRequest::route_stop(spec.route_id()))
                        .await;
                    time::sleep(Duration::from_millis(250)).await;
                    match self.handle_route_intent(route_request).await {
                        Ok(response) => {
                            self.record_route_intent_accepted(
                                spec,
                                job_id,
                                &response,
                                recovery_attempts + 1,
                            )
                            .await
                        }
                        Err(retry_err) => {
                            let message = retry_err.to_string();
                            let blocker = "route_already_running_different_spec".to_string();
                            let job = job_for_phase(spec, job_id, JobPhase::Failed, 100)
                                .with_recovery_attempts(recovery_attempts + 1)
                                .failed(message.clone(), Some(blocker.clone()))
                                .with_next_action("ssh_proxy down --target <target> --json");
                            let job = self
                                .jobs
                                .upsert(job, "route conflict repair failed")
                                .await?;
                            self.state.upsert_session_from_job(spec, &job, None).await?;
                            Ok(RouteReadyOutcome::Failed {
                                blocker,
                                message,
                                remote_url: Some(spec.remote_url()),
                            })
                        }
                    }
                } else {
                    let blocker = state_machine::route_start_blocker(&error);
                    let job = job_for_phase(spec, job_id, JobPhase::Failed, 100)
                        .with_recovery_attempts(recovery_attempts)
                        .failed(error.clone(), Some(blocker.clone()));
                    let job = self.jobs.upsert(job, "route intent failed").await?;
                    self.state.upsert_session_from_job(spec, &job, None).await?;
                    Ok(RouteReadyOutcome::Failed {
                        blocker,
                        message: error,
                        remote_url: Some(spec.remote_url()),
                    })
                }
            }
        }
    }

    async fn record_route_intent_accepted(
        &self,
        spec: &ProxySessionSpec,
        job_id: &str,
        response: &str,
        recovery_attempts: u32,
    ) -> Result<RouteReadyOutcome> {
        let parsed = serde_json::from_str::<Value>(response).unwrap_or(Value::Null);
        let remote_url = parsed
            .get("remote_url")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| Some(spec.remote_url()));
        info!(
            job_id = %job_id,
            session_id = %spec.session_id(),
            route_id = %spec.route_id(),
            peer = %spec.target,
            port = spec.remote_port_policy.preferred,
            remote_url = remote_url.as_deref().unwrap_or("unknown"),
            "proxy session route intent accepted"
        );
        self.jobs
            .upsert(
                job_for_phase(spec, job_id, JobPhase::StartRoute, 70)
                    .with_remote_url(remote_url.clone())
                    .with_recovery_attempts(recovery_attempts),
                "route intent accepted",
            )
            .await?;
        if let Some(job) = self.jobs.get(job_id).await {
            self.state.upsert_session_from_job(spec, &job, None).await?;
        }
        self.wait_for_proxy_route_ready(spec, job_id, remote_url)
            .await
    }

    async fn record_remote_port_range_exhausted(
        &self,
        spec: &ProxySessionSpec,
        job_id: &str,
        failure: PortAttemptFailure,
        candidate_count: usize,
    ) -> Result<()> {
        let candidates = spec.remote_port_policy.candidates();
        let first = candidates
            .first()
            .copied()
            .unwrap_or(spec.remote_port_policy.preferred);
        let last = candidates.last().copied().unwrap_or(first);
        let selected_spec = spec.with_remote_port(failure.port);
        let message = if first == last {
            format!("remote port {first} is unavailable: {}", failure.message)
        } else {
            format!(
                "remote port range {first}-{last} exhausted after {candidate_count} attempts; last failure on {}: {}",
                failure.port, failure.message
            )
        };
        let job = self
            .jobs
            .upsert(
                job_for_phase(&selected_spec, job_id, JobPhase::Failed, 100)
                    .with_remote_url(
                        failure
                            .remote_url
                            .or_else(|| Some(selected_spec.remote_url())),
                    )
                    .with_recovery_attempts(candidate_count.saturating_sub(1) as u32)
                    .failed(
                        message.clone(),
                        Some("remote_port_range_exhausted".to_string()),
                    )
                    .with_next_action(
                        "choose another remote port or increase remote port range size",
                    ),
                "remote port range exhausted",
            )
            .await?;
        self.state
            .upsert_session_from_job(&selected_spec, &job, None)
            .await?;
        warn!(
            job_id = %job_id,
            session_id = %selected_spec.session_id(),
            route_id = %selected_spec.route_id(),
            peer = %selected_spec.target,
            first_port = first,
            last_port = last,
            failed_port = failure.port,
            blocker = %failure.blocker,
            error = %message,
            "remote port range exhausted"
        );
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

#[derive(Debug, Clone)]
struct PortAttemptFailure {
    port: u16,
    blocker: String,
    message: String,
    remote_url: Option<String>,
}

pub(in crate::node_daemon::proxy_session) fn route_request_from_spec(
    spec: &ProxySessionSpec,
) -> NodeRequest {
    let ssh = spec.ssh.as_ref();
    NodeRequest::route_intent(cli::RouteArgs {
        target: spec.target.clone(),
        direction: cli::RouteDirection::RemoteUsesLocal,
        connect_mode: spec.connect_mode.into(),
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
