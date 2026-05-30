use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tracing::{error, warn};

use crate::{cli, deploy, peer_lifecycle, repair};

use super::{
    NodeManager, NodeRequest,
    jobs::{JobPhase, JobRecord, JobState},
    proxy_session::ProxySessionSpec,
    response_line,
    state::PeerStatusRecord,
};

mod job;
mod job_runner;
mod phase_mapping;
mod report;

use job::{remote_peer_job, sanitize_key};
use phase_mapping::{job_phase_from_lifecycle, lifecycle_phase_from_job};
use report::{
    now_unix, peer_status_from_descriptor, remote_dependency_report, remote_peer_lifecycle_report,
    remote_peer_lifecycle_report_record, remote_transport_protocols,
};

impl NodeManager {
    pub(super) async fn remote_peer_ensure(
        self: Arc<Self>,
        request: NodeRequest,
    ) -> Result<String> {
        self.accept_remote_peer_job(
            request,
            "remote_peer_ensure",
            "ensure_remote_peer",
            "remote peer ensure accepted",
        )
        .await
    }

    pub(in crate::node_daemon) async fn accept_remote_peer_job(
        self: Arc<Self>,
        request: NodeRequest,
        response_kind: &'static str,
        job_kind: &'static str,
        accepted_message: &'static str,
    ) -> Result<String> {
        let force_install = request
            .bootstrap
            .as_ref()
            .is_some_and(|bootstrap| bootstrap.force);
        let proxy_session = request.proxy_session.clone();
        let (alias, install_args) = match request.proxy_session.as_ref() {
            Some(spec) => (
                spec.target.clone(),
                self.install_args_from_proxy_session(spec).await?,
            ),
            None => {
                let bootstrap = request.bootstrap.clone().ok_or_else(|| {
                    anyhow!("remote_peer_ensure requires bootstrap or proxy_session")
                })?;
                let alias = bootstrap
                    .alias
                    .clone()
                    .unwrap_or_else(|| bootstrap.target.clone());
                (
                    alias.clone(),
                    self.install_args_from_bootstrap(bootstrap, &alias).await?,
                )
            }
        };
        let job_id = format!("remote-peer:{}", sanitize_key(&alias));
        self.jobs
            .upsert(
                JobRecord::new(job_id.clone(), job_kind)
                    .with_target(alias.clone())
                    .transition(JobState::Queued, JobPhase::Queued, 0),
                accepted_message,
            )
            .await?;
        let manager = self.clone();
        let task_alias = alias.clone();
        let task_job_id = job_id.clone();
        tokio::spawn(async move {
            if let Err(err) = manager
                .run_remote_peer_ensure_job(
                    &task_alias,
                    install_args,
                    &task_job_id,
                    job_kind,
                    proxy_session.as_ref(),
                    force_install,
                )
                .await
            {
                error!(
                    job_id = %task_job_id,
                    peer = %task_alias,
                    error = %err,
                    "remote peer job task failed"
                );
                let failed = remote_peer_job(
                    &task_alias,
                    &task_job_id,
                    job_kind,
                    proxy_session.as_ref(),
                    JobPhase::Failed,
                    100,
                )
                .failed(err.to_string(), Some("job_task_failed".to_string()))
                .with_next_action("run ssh_proxy doctor --json --report");
                let _ = manager
                    .jobs
                    .upsert(failed, "remote peer job task failed")
                    .await;
            }
        });
        let job = self.jobs.get(&job_id).await.map(|job| job.to_value());
        let peer = self
            .state
            .peer_status(&alias)
            .await
            .and_then(|peer| serde_json::to_value(peer).ok());
        response_line(json!({
            "ok": true,
            "kind": response_kind,
            "daemon_api": "v0.3",
            "target": alias.clone(),
            "alias": alias,
            "state": "accepted",
            "job_id": job_id,
            "job": job,
            "peer": peer,
        }))
    }

    pub(super) async fn remote_peer_status(&self, request: NodeRequest) -> Result<String> {
        let target = request
            .alias
            .or(request.id)
            .or_else(|| {
                request
                    .proxy_session
                    .as_ref()
                    .map(|spec| spec.target.clone())
            })
            .or_else(|| request.bootstrap.as_ref().map(|args| args.target.clone()));
        let peer = match target.as_deref() {
            Some(target) => self
                .state
                .peer_status(target)
                .await
                .and_then(|peer| serde_json::to_value(peer).ok())
                .unwrap_or(Value::Null),
            None => self.state.peers_value().await,
        };
        response_line(json!({
            "ok": !peer.is_null(),
            "kind": "remote_peer_status",
            "daemon_api": "v0.3",
            "target": target,
            "peer": peer,
        }))
    }

    pub(super) async fn ensure_remote_peer_for_proxy_session(
        &self,
        spec: &ProxySessionSpec,
        job_id: &str,
    ) -> Result<bool> {
        let install_args = self.install_args_from_proxy_session(spec).await?;
        self.run_remote_peer_ensure_job(
            &spec.target,
            install_args,
            job_id,
            "ensure_proxy_session",
            Some(spec),
            false,
        )
        .await
    }

    async fn install_args_from_proxy_session(
        &self,
        spec: &ProxySessionSpec,
    ) -> Result<cli::InstallRemoteArgs> {
        let mut args = peer_lifecycle::spec::install_args_from_proxy_session(None, spec)?;
        self.apply_daemon_peer_defaults(&mut args, Some(&spec.target))
            .await?;
        Ok(args)
    }

    async fn install_args_from_bootstrap(
        &self,
        bootstrap: cli::PeerBootstrapArgs,
        alias: &str,
    ) -> Result<cli::InstallRemoteArgs> {
        let mut args = peer_lifecycle::spec::install_args_from_peer_bootstrap(bootstrap);
        self.apply_daemon_peer_defaults(&mut args, Some(alias))
            .await?;
        Ok(args)
    }

    async fn apply_daemon_peer_defaults(
        &self,
        args: &mut cli::InstallRemoteArgs,
        alias: Option<&str>,
    ) -> Result<()> {
        let mut config = self.config.lock().await;
        let identity = config.ensure_node_identity()?;
        if config.daemon.control_endpoint.is_none() && config.daemon.control_listen.is_none() {
            config.daemon.control_endpoint = Some(self.control_endpoint.to_string());
        }
        if config.daemon.transport_listen.is_none() {
            config.daemon.transport_listen = self.transport;
        }
        if config.daemon.token.is_none() {
            config.ensure_daemon_token()?;
        }
        args.local_node_id = identity.node_id;
        args.local_node_name = identity.node_name;
        args.local_control_endpoint = Some(self.control_endpoint.to_string());
        args.local_transport = self.transport;
        args.persist = cli::PersistMode::Auto;
        crate::config::apply_install_defaults(&config, args, alias)?;
        config.save_default()
    }

    async fn remote_peer_phase(
        &self,
        alias: &str,
        job_id: &str,
        job_kind: &str,
        spec: Option<&ProxySessionSpec>,
        install_args: &cli::InstallRemoteArgs,
        phase: JobPhase,
        _progress: u8,
        message: &str,
    ) -> Result<JobRecord> {
        let report = remote_peer_lifecycle_report_record(
            alias,
            lifecycle_phase_from_job(phase),
            peer_lifecycle::workflow::LifecycleOperation::Ensure,
            Some(install_args),
            install_args.remote_path.as_deref(),
            "auto",
            None,
            None,
            0,
        );
        let mut sink = RemotePeerLifecycleSink {
            manager: self,
            alias,
            job_id,
            job_kind,
            spec,
        };
        peer_lifecycle::workflow::LifecycleEventSink::emit(
            &mut sink,
            peer_lifecycle::workflow::LifecycleEvent {
                operation: peer_lifecycle::workflow::LifecycleOperation::Ensure,
                report,
                message: message.to_string(),
            },
        )
        .await;
        self.jobs
            .get(job_id)
            .await
            .ok_or_else(|| anyhow!("remote peer lifecycle event did not record job {job_id}"))
    }

    async fn record_peer_waiting(
        &self,
        alias: &str,
        job_id: &str,
        job_kind: &str,
        spec: Option<&ProxySessionSpec>,
        install_args: &cli::InstallRemoteArgs,
        last_error: Option<String>,
        message: &str,
    ) -> Result<()> {
        let job = self
            .jobs
            .upsert(
                remote_peer_job(
                    alias,
                    job_id,
                    job_kind,
                    spec,
                    JobPhase::InspectPeerDescriptor,
                    33,
                )
                .transition(JobState::WaitingRetry, JobPhase::InspectPeerDescriptor, 33)
                .with_next_action("bootstrap_remote_peer")
                .with_retry_after_ms(250),
                message,
            )
            .await?;
        if let Some(spec) = spec {
            self.state.upsert_session_from_job(spec, &job, None).await?;
        }
        if let Some(existing) = self.state.peer_status(alias).await {
            self.state
                .upsert_peer_status(PeerStatusRecord {
                    install: Some(remote_peer_lifecycle_report(
                        alias,
                        peer_lifecycle::workflow::PeerLifecyclePhase::InspectDescriptor,
                        peer_lifecycle::workflow::LifecycleOperation::Ensure,
                        Some(install_args),
                        install_args.remote_path.as_deref(),
                        "auto",
                        None,
                        last_error.as_deref(),
                        existing.recovery_attempts,
                    )),
                    last_error,
                    retry_after_ms: Some(250),
                    ..existing
                })
                .await?;
        }
        Ok(())
    }

    async fn record_refreshed_peer(
        &self,
        alias: &str,
        result: &deploy::RemoteDescriptorResult,
        install_args: &cli::InstallRemoteArgs,
        job_id: &str,
        job_kind: &str,
        spec: Option<&ProxySessionSpec>,
    ) -> Result<()> {
        {
            let mut config = self.config.lock().await;
            deploy::record_remote_descriptor_profile(&mut config, alias, result)?;
        }
        let job = self
            .jobs
            .upsert(
                remote_peer_job(alias, job_id, job_kind, spec, JobPhase::RecordPeer, 44)
                    .with_next_action("continue_proxy_session"),
                "persistent remote peer descriptor adopted",
            )
            .await?;
        if let Some(spec) = spec {
            self.state.upsert_session_from_job(spec, &job, None).await?;
        }
        self.state
            .upsert_peer_status(peer_status_from_descriptor(
                alias,
                &result.descriptor,
                result.remote_control,
                result.remote_tcp,
                "adopted",
                "inspect_descriptor",
                "unknown",
                0,
                Some(install_args),
                Some(&result.remote_path),
                peer_lifecycle::workflow::LifecycleOperation::Ensure,
            ))
            .await?;
        self.finish_direct_remote_peer_job(alias, job_id, job_kind, spec)
            .await?;
        Ok(())
    }

    async fn record_installed_peer(
        &self,
        alias: &str,
        result: &deploy::RemoteInstallResult,
        install_args: &cli::InstallRemoteArgs,
        job_id: &str,
        job_kind: &str,
        spec: Option<&ProxySessionSpec>,
    ) -> Result<()> {
        {
            let mut config = self.config.lock().await;
            deploy::record_remote_install_profile(&mut config, alias, result)?;
        }
        let job = self
            .jobs
            .upsert(
                remote_peer_job(alias, job_id, job_kind, spec, JobPhase::RecordPeer, 44)
                    .with_next_action("continue_proxy_session"),
                "persistent remote peer installed and recorded",
            )
            .await?;
        if let Some(spec) = spec {
            self.state.upsert_session_from_job(spec, &job, None).await?;
        }
        let descriptor = result.descriptor.as_ref().cloned().unwrap_or_else(|| {
            json!({
                "version": env!("CARGO_PKG_VERSION"),
                "transport_protocols": remote_transport_protocols(result),
                "endpoints": {
                    "control": format!("tcp://{}", result.remote_control),
                    "transport": result.remote_tcp.to_string(),
                }
            })
        });
        let mut status = peer_status_from_descriptor(
            alias,
            &descriptor,
            result.remote_control,
            result.remote_tcp,
            "healthy",
            "start_service",
            &result.service_manager,
            0,
            Some(install_args),
            Some(&result.remote_path),
            peer_lifecycle::workflow::LifecycleOperation::Ensure,
        );
        if let Some(install_report) = &result.install_report {
            status.install = Some(install_report.clone());
        }
        self.state.upsert_peer_status(status).await?;
        self.finish_direct_remote_peer_job(alias, job_id, job_kind, spec)
            .await?;
        Ok(())
    }

    async fn finish_direct_remote_peer_job(
        &self,
        alias: &str,
        job_id: &str,
        job_kind: &str,
        spec: Option<&ProxySessionSpec>,
    ) -> Result<()> {
        if spec.is_some() {
            return Ok(());
        }
        self.jobs
            .upsert(
                remote_peer_job(alias, job_id, job_kind, None, JobPhase::Healthy, 100).transition(
                    JobState::Healthy,
                    JobPhase::Healthy,
                    100,
                ),
                "remote peer ready",
            )
            .await?;
        Ok(())
    }
}

struct RemotePeerLifecycleSink<'a> {
    manager: &'a NodeManager,
    alias: &'a str,
    job_id: &'a str,
    job_kind: &'a str,
    spec: Option<&'a ProxySessionSpec>,
}

impl peer_lifecycle::workflow::LifecycleEventSink for RemotePeerLifecycleSink<'_> {
    fn emit<'a>(
        &'a mut self,
        event: peer_lifecycle::workflow::LifecycleEvent,
    ) -> peer_lifecycle::workflow::BoxEventFuture<'a> {
        Box::pin(async move {
            if let Err(err) = self.record_event(event).await {
                warn!(
                    error = %err,
                    job_id = %self.job_id,
                    alias = %self.alias,
                    "failed to record remote peer lifecycle event"
                );
            }
        })
    }
}

impl RemotePeerLifecycleSink<'_> {
    async fn record_event(&self, event: peer_lifecycle::workflow::LifecycleEvent) -> Result<()> {
        let phase = job_phase_from_lifecycle(event.report.phase);
        let progress = event.report.phase.progress();
        let mut job = remote_peer_job(
            self.alias,
            self.job_id,
            self.job_kind,
            self.spec,
            phase,
            progress,
        )
        .with_next_action("wait_for_remote_peer");
        if event.report.phase == peer_lifecycle::workflow::PeerLifecyclePhase::Failed {
            job = job.failed(
                event
                    .report
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "remote peer lifecycle failed".to_string()),
                event.report.blocker.clone(),
            );
        }
        let job = self.manager.jobs.upsert(job, &event.message).await?;
        if let Some(spec) = self.spec {
            self.manager
                .state
                .upsert_session_from_job(spec, &job, None)
                .await?;
        }
        let failed = event.report.phase == peer_lifecycle::workflow::PeerLifecyclePhase::Failed;
        self.manager
            .state
            .upsert_peer_status(PeerStatusRecord {
                target: self.alias.to_string(),
                state: if failed { "failed" } else { "running" }.to_string(),
                health: if failed { "failed" } else { "starting" }.to_string(),
                version: None,
                control_endpoint: None,
                transport: None,
                transport_protocols: Vec::new(),
                service_manager: event
                    .report
                    .service_manager
                    .clone()
                    .or_else(|| Some("auto".to_string())),
                descriptor_hash: None,
                install: Some(event.report.to_redacted_value()),
                dependency_report: Some(remote_dependency_report()),
                update_required: false,
                blocker: event.report.blocker.clone(),
                repair_action: event
                    .report
                    .blocker
                    .as_deref()
                    .and_then(repair::action_for_blocker),
                last_error: event.report.last_error.clone(),
                retry_after_ms: event.report.retry_after_ms,
                recovery_attempts: event.report.recovery_attempts,
                updated_at_unix: now_unix(),
            })
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::report::remote_peer_blocker;
    use super::*;

    fn install_args() -> cli::InstallRemoteArgs {
        cli::InstallRemoteArgs {
            target: "edge".to_string(),
            ssh_args: Vec::new(),
            ssh_command: None,
            user: None,
            port: None,
            identity: Vec::new(),
            config: None,
            known_hosts: None,
            accept_new: false,
            insecure_ignore_host_key: false,
            jump: Vec::new(),
            remote_path: None,
            remote_bin: None,
            remote_os: cli::RemoteOs::Unix,
            remote_token: Some("secret".to_string()),
            remote_tcp: "127.0.0.1:19080".parse().unwrap(),
            remote_control: "127.0.0.1:19081".parse().unwrap(),
            local_node_id: None,
            local_node_name: None,
            local_control_endpoint: None,
            local_transport: None,
            remote_node_id: None,
            remote_node_name: None,
            remote_tls_transport: None,
            remote_quic_transport: None,
            remote_tls_cert: None,
            remote_tls_key: None,
            remote_tls_client_ca: None,
            persist: cli::PersistMode::Systemd,
        }
    }

    #[test]
    fn descriptor_status_records_hash_and_protocols() {
        let descriptor = json!({
            "version": env!("CARGO_PKG_VERSION"),
            "transport_protocols": ["tls-tcp", "plain-tcp"],
            "endpoints": {
                "control": "tcp://127.0.0.1:19081",
                "transport": "127.0.0.1:19080"
            }
        });
        let args = install_args();
        let status = peer_status_from_descriptor(
            "edge",
            &descriptor,
            "127.0.0.1:19081".parse().unwrap(),
            "127.0.0.1:19080".parse().unwrap(),
            "healthy",
            "start_service",
            "systemd_user",
            1,
            Some(&args),
            Some("/home/me/bin/ssh_proxy"),
            peer_lifecycle::workflow::LifecycleOperation::Ensure,
        );
        assert_eq!(status.health, "healthy");
        assert_eq!(status.service_manager.as_deref(), Some("systemd_user"));
        assert_eq!(status.transport_protocols, vec!["tls-tcp", "plain-tcp"]);
        assert!(status.descriptor_hash.is_some());
        assert!(!status.update_required);
        assert_eq!(status.recovery_attempts, 1);
        let install = status.install.as_ref().unwrap();
        assert_eq!(install["role"], "remote_peer");
        assert_eq!(install["platform"], "linux");
        assert_eq!(install["operation"], "ensure");
        assert_eq!(install["provider"], "systemd_user");
        assert_eq!(install["service_name"], "ssh-proxy-helper");
    }

    #[test]
    fn descriptor_update_required_detects_stale_or_missing_version() {
        assert!(!report::descriptor_update_required(&json!({
            "version": env!("CARGO_PKG_VERSION")
        })));
        assert!(report::descriptor_install_required(
            &json!({
                "version": env!("CARGO_PKG_VERSION")
            }),
            true
        ));
        assert!(!report::descriptor_install_required(
            &json!({
                "version": env!("CARGO_PKG_VERSION")
            }),
            false
        ));
        assert!(report::descriptor_update_required(&json!({
            "version": "0.0.1"
        })));
        assert!(report::descriptor_update_required(&json!({})));
        assert!(report::descriptor_version_message(&json!({"version": "0.0.1"})).contains("0.0.1"));
    }

    #[test]
    fn remote_peer_errors_are_classified() {
        assert_eq!(
            remote_peer_blocker("SSH authentication failed: no accepted identity"),
            "ssh_auth_failed"
        );
        assert_eq!(
            remote_peer_blocker("ProxyCommand is unsupported"),
            "ssh_config_unsupported"
        );
        assert_eq!(remote_peer_blocker("boom"), "remote_peer_install_failed");
    }
}
