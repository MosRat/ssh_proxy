use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{cli, deploy, peer_lifecycle, repair};

use super::{
    NodeManager, NodeRequest,
    jobs::{JobPhase, JobRecord, JobState},
    proxy_session::ProxySessionSpec,
    response_line,
    state::PeerStatusRecord,
};

impl NodeManager {
    pub(super) async fn remote_peer_ensure(&self, request: NodeRequest) -> Result<String> {
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
                JobRecord::new(job_id.clone(), "ensure_remote_peer")
                    .with_target(alias.clone())
                    .transition(JobState::Queued, JobPhase::Queued, 0),
                "remote peer ensure accepted",
            )
            .await?;
        let ok = self
            .run_remote_peer_ensure_job(
                &alias,
                install_args,
                &job_id,
                "ensure_remote_peer",
                request.proxy_session.as_ref(),
            )
            .await?;
        let job = self.jobs.get(&job_id).await.map(|job| job.to_value());
        let peer = self
            .state
            .peer_status(&alias)
            .await
            .and_then(|peer| serde_json::to_value(peer).ok());
        response_line(json!({
            "ok": ok,
            "kind": "remote_peer_ensure",
            "daemon_api": "v0.3",
            "target": alias,
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
        config.apply_install_defaults(args, alias)?;
        config.save_default()
    }

    async fn run_remote_peer_ensure_job(
        &self,
        alias: &str,
        install_args: cli::InstallRemoteArgs,
        job_id: &str,
        job_kind: &str,
        spec: Option<&ProxySessionSpec>,
    ) -> Result<bool> {
        self.remote_peer_phase(
            alias,
            job_id,
            job_kind,
            spec,
            JobPhase::InspectPeerDescriptor,
            32,
            "inspecting persistent remote peer descriptor",
        )
        .await?;
        match deploy::refresh_remote_peer_descriptor(install_args.clone()).await {
            Ok(result) => {
                self.record_refreshed_peer(alias, &result, job_id, job_kind, spec)
                    .await?;
                return Ok(true);
            }
            Err(err) => {
                self.record_peer_waiting(
                    alias,
                    job_id,
                    job_kind,
                    spec,
                    Some(format!("{err:#}")),
                    "remote descriptor unavailable; bootstrapping persistent peer",
                )
                .await?;
            }
        }

        for (phase, progress, message) in [
            (
                JobPhase::DependencyCheck,
                34,
                "checking remote peer dependencies",
            ),
            (JobPhase::StageRemotePeer, 36, "staging remote peer binary"),
            (
                JobPhase::WritePeerConfig,
                38,
                "writing remote peer configuration",
            ),
            (
                JobPhase::InstallPeerService,
                40,
                "installing remote peer service",
            ),
        ] {
            self.remote_peer_phase(alias, job_id, job_kind, spec, phase, progress, message)
                .await?;
        }

        match deploy::install_remote(install_args).await {
            Ok(result) => {
                self.remote_peer_phase(
                    alias,
                    job_id,
                    job_kind,
                    spec,
                    JobPhase::PeerHealthProbe,
                    42,
                    "remote peer service answered health probe",
                )
                .await?;
                self.record_installed_peer(alias, &result, job_id, job_kind, spec)
                    .await?;
                Ok(true)
            }
            Err(err) => {
                let error = format!("{err:#}");
                let blocker = remote_peer_blocker(&error);
                let job = remote_peer_job(alias, job_id, job_kind, spec, JobPhase::Failed, 100)
                    .failed(error.clone(), Some(blocker.clone()))
                    .with_next_action("run ssh_proxy doctor --json --report");
                let job = self.jobs.upsert(job, "remote peer ensure failed").await?;
                if let Some(spec) = spec {
                    self.state.upsert_session_from_job(spec, &job, None).await?;
                }
                self.state
                    .upsert_peer_status(PeerStatusRecord {
                        target: alias.to_string(),
                        state: "failed".to_string(),
                        health: "failed".to_string(),
                        version: None,
                        control_endpoint: None,
                        transport: None,
                        transport_protocols: Vec::new(),
                        service_manager: Some("auto".to_string()),
                        descriptor_hash: None,
                        install: Some(remote_peer_lifecycle_report(
                            alias,
                            peer_lifecycle::workflow::PeerLifecyclePhase::Failed,
                            "auto",
                            Some(&blocker),
                            Some(&error),
                            0,
                        )),
                        dependency_report: Some(remote_dependency_report()),
                        update_required: false,
                        blocker: Some(blocker.clone()),
                        repair_action: repair::action_for_blocker(&blocker),
                        last_error: Some(error),
                        retry_after_ms: Some(1000),
                        recovery_attempts: 0,
                        updated_at_unix: now_unix(),
                    })
                    .await?;
                Ok(false)
            }
        }
    }

    async fn remote_peer_phase(
        &self,
        alias: &str,
        job_id: &str,
        job_kind: &str,
        spec: Option<&ProxySessionSpec>,
        phase: JobPhase,
        progress: u8,
        message: &str,
    ) -> Result<JobRecord> {
        let job = self
            .jobs
            .upsert(
                remote_peer_job(alias, job_id, job_kind, spec, phase, progress)
                    .with_next_action("wait_for_remote_peer"),
                message,
            )
            .await?;
        if let Some(spec) = spec {
            self.state.upsert_session_from_job(spec, &job, None).await?;
        }
        self.state
            .upsert_peer_status(PeerStatusRecord {
                target: alias.to_string(),
                state: "running".to_string(),
                health: "starting".to_string(),
                version: None,
                control_endpoint: None,
                transport: None,
                transport_protocols: Vec::new(),
                service_manager: Some("auto".to_string()),
                descriptor_hash: None,
                install: Some(remote_peer_lifecycle_report(
                    alias,
                    lifecycle_phase_from_job(phase),
                    "auto",
                    None,
                    None,
                    0,
                )),
                dependency_report: Some(remote_dependency_report()),
                update_required: false,
                blocker: None,
                repair_action: None,
                last_error: None,
                retry_after_ms: None,
                recovery_attempts: 0,
                updated_at_unix: now_unix(),
            })
            .await?;
        Ok(job)
    }

    async fn record_peer_waiting(
        &self,
        alias: &str,
        job_id: &str,
        job_kind: &str,
        spec: Option<&ProxySessionSpec>,
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
            ))
            .await?;
        Ok(())
    }

    async fn record_installed_peer(
        &self,
        alias: &str,
        result: &deploy::RemoteInstallResult,
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
        self.state
            .upsert_peer_status(peer_status_from_descriptor(
                alias,
                &descriptor,
                result.remote_control,
                result.remote_tcp,
                "healthy",
                "start_service",
                &result.service_manager,
                0,
            ))
            .await?;
        Ok(())
    }
}

fn remote_peer_job(
    alias: &str,
    job_id: &str,
    kind: &str,
    spec: Option<&ProxySessionSpec>,
    phase: JobPhase,
    progress: u8,
) -> JobRecord {
    let mut job = JobRecord::new(job_id.to_string(), kind.to_string())
        .with_target(alias.to_string())
        .transition(JobState::Running, phase, progress);
    if let Some(spec) = spec {
        job = job
            .with_workspace(spec.workspace_id.clone())
            .with_route(spec.route_id())
            .with_remote_url(Some(spec.remote_url()));
    }
    job
}

fn peer_status_from_descriptor(
    alias: &str,
    descriptor: &Value,
    remote_control: std::net::SocketAddr,
    remote_tcp: std::net::SocketAddr,
    install_state: &str,
    install_phase: &str,
    service_manager: &str,
    recovery_attempts: u32,
) -> PeerStatusRecord {
    let protocols = descriptor
        .get("transport_protocols")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| vec!["plain-tcp".to_string()]);
    PeerStatusRecord {
        target: alias.to_string(),
        state: "healthy".to_string(),
        health: "healthy".to_string(),
        version: descriptor
            .get("version")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        control_endpoint: descriptor
            .pointer("/endpoints/control")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| Some(format!("tcp://{remote_control}"))),
        transport: descriptor
            .pointer("/endpoints/transport")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| Some(remote_tcp.to_string())),
        transport_protocols: protocols,
        service_manager: Some(service_manager.to_string()),
        descriptor_hash: Some(value_hash(descriptor)),
        install: Some(remote_peer_lifecycle_report(
            alias,
            lifecycle_phase_from_install_state(install_state, install_phase),
            service_manager,
            None,
            None,
            recovery_attempts,
        )),
        dependency_report: Some(remote_dependency_report()),
        update_required: descriptor
            .get("version")
            .and_then(Value::as_str)
            .is_some_and(|version| version != env!("CARGO_PKG_VERSION")),
        blocker: None,
        repair_action: None,
        last_error: None,
        retry_after_ms: None,
        recovery_attempts,
        updated_at_unix: now_unix(),
    }
}

fn remote_dependency_report() -> Value {
    json!([
        {
            "name": "remote_posix_shell",
            "classification": "required",
            "state": "checked_during_peer_ensure",
            "message": "Linux/macOS remote peer install uses a POSIX shell for service setup"
        },
        {
            "name": "remote_systemd_user",
            "classification": "optional",
            "state": "preferred_on_linux",
            "message": "user systemd is the preferred Linux remote peer service manager"
        },
        {
            "name": "remote_nohup_supervisor",
            "classification": "optional",
            "state": "fallback_on_linux",
            "message": "managed nohup supervisor is used when user systemd is unavailable"
        },
        {
            "name": "remote_launchd_user",
            "classification": "optional",
            "state": "macos_provider",
            "message": "LaunchAgent provider is used for macOS remotes"
        },
        {
            "name": "remote_schtasks_user",
            "classification": "optional",
            "state": "windows_provider",
            "message": "scheduled task provider is used for Windows user-scope remotes"
        }
    ])
}

fn remote_peer_blocker(error: &str) -> String {
    if error.contains("SSH authentication failed") {
        "ssh_auth_failed".to_string()
    } else if error.contains("ProxyCommand")
        || error.contains("unsupported --ssh-arg")
        || error.contains("unsupported -o")
    {
        "ssh_config_unsupported".to_string()
    } else {
        "remote_peer_install_failed".to_string()
    }
}

fn remote_transport_protocols(result: &deploy::RemoteInstallResult) -> Vec<String> {
    let mut protocols = Vec::new();
    if result.remote_quic_transport.is_some() {
        protocols.push("quic".to_string());
    }
    if result.remote_tls_transport.is_some() {
        protocols.push("tls-tcp".to_string());
    }
    protocols.push("plain-tcp".to_string());
    protocols
}

fn value_hash(value: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.to_string().as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn lifecycle_phase_from_job(phase: JobPhase) -> peer_lifecycle::workflow::PeerLifecyclePhase {
    match phase {
        JobPhase::InspectPeerDescriptor => {
            peer_lifecycle::workflow::PeerLifecyclePhase::InspectDescriptor
        }
        JobPhase::DependencyCheck => peer_lifecycle::workflow::PeerLifecyclePhase::DependencyCheck,
        JobPhase::StageRemotePeer => peer_lifecycle::workflow::PeerLifecyclePhase::StageBinary,
        JobPhase::WritePeerConfig => peer_lifecycle::workflow::PeerLifecyclePhase::WriteConfig,
        JobPhase::InstallPeerService => {
            peer_lifecycle::workflow::PeerLifecyclePhase::InstallService
        }
        JobPhase::StartPeerService => peer_lifecycle::workflow::PeerLifecyclePhase::StartService,
        JobPhase::PeerHealthProbe => peer_lifecycle::workflow::PeerLifecyclePhase::HealthProbe,
        JobPhase::RecordPeer => peer_lifecycle::workflow::PeerLifecyclePhase::Record,
        JobPhase::Failed => peer_lifecycle::workflow::PeerLifecyclePhase::Failed,
        _ => peer_lifecycle::workflow::PeerLifecyclePhase::Prepare,
    }
}

fn lifecycle_phase_from_install_state(
    install_state: &str,
    install_phase: &str,
) -> peer_lifecycle::workflow::PeerLifecyclePhase {
    if install_state == "healthy" {
        return peer_lifecycle::workflow::PeerLifecyclePhase::Healthy;
    }
    match install_phase {
        "inspect_descriptor" => peer_lifecycle::workflow::PeerLifecyclePhase::InspectDescriptor,
        "dependency_check" => peer_lifecycle::workflow::PeerLifecyclePhase::DependencyCheck,
        "stage_binary" => peer_lifecycle::workflow::PeerLifecyclePhase::StageBinary,
        "write_config" => peer_lifecycle::workflow::PeerLifecyclePhase::WriteConfig,
        "install_service" => peer_lifecycle::workflow::PeerLifecyclePhase::InstallService,
        "start_service" => peer_lifecycle::workflow::PeerLifecyclePhase::StartService,
        "health_probe" => peer_lifecycle::workflow::PeerLifecyclePhase::HealthProbe,
        "record_peer" | "record" => peer_lifecycle::workflow::PeerLifecyclePhase::Record,
        "failed" => peer_lifecycle::workflow::PeerLifecyclePhase::Failed,
        _ => peer_lifecycle::workflow::PeerLifecyclePhase::Prepare,
    }
}

fn remote_peer_lifecycle_report(
    alias: &str,
    phase: peer_lifecycle::workflow::PeerLifecyclePhase,
    service_manager: &str,
    blocker: Option<&str>,
    last_error: Option<&str>,
    recovery_attempts: u32,
) -> Value {
    let mut report = peer_lifecycle::report::PeerLifecycleReport::new(alias, phase);
    report.service_manager = Some(service_manager.to_string());
    report.recovery_attempts = recovery_attempts;
    report.blocker = blocker.map(ToOwned::to_owned);
    report.last_error = last_error.map(ToOwned::to_owned);
    report.to_redacted_value()
}

fn sanitize_key(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let status = peer_status_from_descriptor(
            "edge",
            &descriptor,
            "127.0.0.1:19081".parse().unwrap(),
            "127.0.0.1:19080".parse().unwrap(),
            "healthy",
            "start_service",
            "systemd_user",
            1,
        );
        assert_eq!(status.health, "healthy");
        assert_eq!(status.service_manager.as_deref(), Some("systemd_user"));
        assert_eq!(status.transport_protocols, vec!["tls-tcp", "plain-tcp"]);
        assert!(status.descriptor_hash.is_some());
        assert!(!status.update_required);
        assert_eq!(status.recovery_attempts, 1);
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
