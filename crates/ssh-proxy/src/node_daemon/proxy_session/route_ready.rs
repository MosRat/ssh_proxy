use std::time::Duration;

use anyhow::Result;
use serde_json::Value;
use tokio::time;

use crate::node_daemon::{
    NodeManager,
    handoff::{self, HandoffProbeStatus},
    jobs::{JobPhase, JobState},
    remote_setup,
    state::RemoteSetupStatus,
};

use super::{ProxySessionSpec, error_chain, job_runner::job_for_phase, status};

#[derive(Debug, Clone)]
pub(super) enum RouteReadyOutcome {
    Healthy,
    Failed {
        blocker: String,
        message: String,
        remote_url: Option<String>,
    },
}

impl NodeManager {
    pub(super) async fn wait_for_proxy_route_ready(
        &self,
        spec: &ProxySessionSpec,
        job_id: &str,
        remote_url: Option<String>,
    ) -> Result<RouteReadyOutcome> {
        let route_id = spec.route_id();
        let deadline = time::Instant::now() + Duration::from_secs(90);
        loop {
            let status_value = self.status_value().await?;
            if let Some(route) = status::find_route(&status_value, &route_id) {
                let state = status::route_state(&route);
                if matches!(state.as_deref(), Some("error" | "failed")) {
                    let error = route
                        .get("last_error")
                        .and_then(Value::as_str)
                        .unwrap_or("route failed")
                        .to_string();
                    let job = job_for_phase(spec, job_id, JobPhase::Failed, 100)
                        .with_remote_url(remote_url.clone())
                        .failed(error.clone(), Some("route_failed".to_string()));
                    let job = self.jobs.upsert(job, "route failed").await?;
                    self.state
                        .upsert_session_from_job(spec, &job, Some(route))
                        .await?;
                    return Ok(RouteReadyOutcome::Failed {
                        blocker: "route_failed".to_string(),
                        message: error,
                        remote_url,
                    });
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
                                let blocker = failure.blocker.clone();
                                let message = failure.message.clone();
                                let job = job_for_phase(spec, job_id, JobPhase::Failed, 100)
                                    .with_remote_url(Some(remote_url_value.clone()))
                                    .failed(message.clone(), Some(blocker.clone()))
                                    .with_next_action("run ssh_proxy doctor --json");
                                let job =
                                    self.jobs.upsert(job, "remote handoff probe failed").await?;
                                self.state
                                    .upsert_session_from_job(spec, &job, Some(route.clone()))
                                    .await?;
                                return Ok(RouteReadyOutcome::Failed {
                                    blocker,
                                    message,
                                    remote_url: Some(remote_url_value),
                                });
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
                                    RemoteSetupStatus::failed(
                                        error.clone(),
                                        None,
                                        Some(remote_url_value.clone()),
                                    ),
                                )
                                .await?;
                            return Ok(RouteReadyOutcome::Failed {
                                blocker: "remote_setup_failed".to_string(),
                                message: error,
                                remote_url: Some(remote_url_value.clone()),
                            });
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
                    return Ok(RouteReadyOutcome::Healthy);
                }
            }
            if time::Instant::now() >= deadline {
                let remote_url_for_failure = remote_url.clone();
                let job = job_for_phase(spec, job_id, JobPhase::Failed, 100)
                    .with_remote_url(remote_url)
                    .failed(
                        "route readiness timed out before remote handoff could start",
                        Some("route_ready_timeout".to_string()),
                    )
                    .with_next_action("rerun_ensure_proxy_session");
                let job = self.jobs.upsert(job, "route readiness timed out").await?;
                self.state.upsert_session_from_job(spec, &job, None).await?;
                return Ok(RouteReadyOutcome::Failed {
                    blocker: "route_ready_timeout".to_string(),
                    message: "route readiness timed out before remote handoff could start"
                        .to_string(),
                    remote_url: remote_url_for_failure,
                });
            }
            time::sleep(Duration::from_millis(250)).await;
        }
    }
}
