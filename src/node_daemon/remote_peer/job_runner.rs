use anyhow::Result;

use crate::{cli, deploy, peer_lifecycle, repair};

use crate::node_daemon::{
    NodeManager, jobs::JobPhase, proxy_session::ProxySessionSpec, state::PeerStatusRecord,
};

use super::{
    job::remote_peer_job,
    report::{
        now_unix, remote_dependency_report, remote_peer_blocker, remote_peer_lifecycle_report,
    },
};

impl NodeManager {
    pub(super) async fn run_remote_peer_ensure_job(
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
            &install_args,
            JobPhase::InspectPeerDescriptor,
            32,
            "inspecting persistent remote peer descriptor",
        )
        .await?;
        match deploy::refresh_remote_peer_descriptor(install_args.clone()).await {
            Ok(result) => {
                self.record_refreshed_peer(alias, &result, &install_args, job_id, job_kind, spec)
                    .await?;
                return Ok(true);
            }
            Err(err) => {
                self.record_peer_waiting(
                    alias,
                    job_id,
                    job_kind,
                    spec,
                    &install_args,
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
            self.remote_peer_phase(
                alias,
                job_id,
                job_kind,
                spec,
                &install_args,
                phase,
                progress,
                message,
            )
            .await?;
        }

        match deploy::install_remote(install_args.clone()).await {
            Ok(result) => {
                self.remote_peer_phase(
                    alias,
                    job_id,
                    job_kind,
                    spec,
                    &install_args,
                    JobPhase::PeerHealthProbe,
                    42,
                    "remote peer service answered health probe",
                )
                .await?;
                self.record_installed_peer(alias, &result, &install_args, job_id, job_kind, spec)
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
                            peer_lifecycle::workflow::LifecycleOperation::Ensure,
                            Some(&install_args),
                            install_args.remote_path.as_deref(),
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
}
