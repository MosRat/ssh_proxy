use serde::{Deserialize, Serialize};

use anyhow::{Context, Result, bail};

use super::{executor::PeerExecutor, report::PeerLifecycleReport, spec::PeerLifecycleSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PeerLifecyclePhase {
    Prepare,
    InspectDescriptor,
    DependencyCheck,
    StageBinary,
    WriteConfig,
    InstallService,
    StartService,
    HealthProbe,
    Record,
    Healthy,
    Repairing,
    Rollback,
    Failed,
}

impl PeerLifecyclePhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::InspectDescriptor => "inspect_descriptor",
            Self::DependencyCheck => "dependency_check",
            Self::StageBinary => "stage_binary",
            Self::WriteConfig => "write_config",
            Self::InstallService => "install_service",
            Self::StartService => "start_service",
            Self::HealthProbe => "health_probe",
            Self::Record => "record",
            Self::Healthy => "healthy",
            Self::Repairing => "repairing",
            Self::Rollback => "rollback",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn progress(self) -> u8 {
        match self {
            Self::Prepare => 5,
            Self::InspectDescriptor => 15,
            Self::DependencyCheck => 25,
            Self::StageBinary => 35,
            Self::WriteConfig => 45,
            Self::InstallService => 60,
            Self::StartService => 72,
            Self::HealthProbe => 85,
            Self::Record => 95,
            Self::Healthy => 100,
            Self::Repairing => 50,
            Self::Rollback => 90,
            Self::Failed => 100,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LifecycleCommand {
    pub(crate) phase: PeerLifecyclePhase,
    pub(crate) command: String,
    pub(crate) stdin: Option<Vec<u8>>,
}

impl LifecycleCommand {
    pub(crate) fn new(phase: PeerLifecyclePhase, command: impl Into<String>) -> Self {
        Self {
            phase,
            command: command.into(),
            stdin: None,
        }
    }

    pub(crate) fn with_stdin(mut self, stdin: Vec<u8>) -> Self {
        self.stdin = Some(stdin);
        self
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PeerLifecycleWorkflowResult {
    pub(crate) report: PeerLifecycleReport,
    pub(crate) phase_reports: Vec<PeerLifecycleReport>,
}

pub(crate) async fn run_lifecycle_commands<E: PeerExecutor>(
    executor: &E,
    spec: &PeerLifecycleSpec,
    commands: Vec<LifecycleCommand>,
) -> Result<PeerLifecycleWorkflowResult> {
    let mut phase_reports = Vec::new();
    phase_reports.push(phase_report(spec, PeerLifecyclePhase::Prepare));
    for command in commands {
        let mut report = phase_report(spec, command.phase);
        let output = executor
            .exec_capture(command.command.clone(), command.stdin)
            .await
            .with_context(|| format!("failed to run lifecycle phase {}", command.phase.as_str()))?;
        if output.exit_status != 0 {
            let error = if output.stderr.trim().is_empty() {
                output.stdout.trim().to_string()
            } else {
                output.stderr.trim().to_string()
            };
            report.state = PeerLifecyclePhase::Failed.as_str().to_string();
            report.phase = PeerLifecyclePhase::Failed;
            report.blocker = Some(format!("{}_failed", command.phase.as_str()));
            report.last_error = Some(error.clone());
            phase_reports.push(report);
            bail!(
                "lifecycle phase {} failed with status {}: {}",
                command.phase.as_str(),
                output.exit_status,
                error
            );
        }
        phase_reports.push(report);
    }
    let report = phase_report(spec, PeerLifecyclePhase::Healthy);
    phase_reports.push(report.clone());
    Ok(PeerLifecycleWorkflowResult {
        report,
        phase_reports,
    })
}

pub(crate) fn phase_report(
    spec: &PeerLifecycleSpec,
    phase: PeerLifecyclePhase,
) -> PeerLifecycleReport {
    let mut report = PeerLifecycleReport::new(spec.target.clone(), phase);
    report.service_manager = Some(spec.provider.manager_name().to_string());
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_phase_names_match_public_json_contract() {
        assert_eq!(PeerLifecyclePhase::Prepare.as_str(), "prepare");
        assert_eq!(
            PeerLifecyclePhase::InspectDescriptor.as_str(),
            "inspect_descriptor"
        );
        assert_eq!(PeerLifecyclePhase::HealthProbe.progress(), 85);
        assert_eq!(PeerLifecyclePhase::Failed.progress(), 100);
    }

    #[tokio::test]
    async fn lifecycle_workflow_runs_commands_and_reports_healthy() {
        let executor = crate::peer_lifecycle::executor::FakeExecutor::default();
        executor.push_output(crate::ssh_client::ExecOutput {
            exit_status: 0,
            stdout: "ok".to_string(),
            stderr: String::new(),
        });
        let spec = crate::peer_lifecycle::spec::PeerLifecycleSpec::local_daemon(
            "local",
            "ssh_proxy",
            crate::peer_lifecycle::service_provider::ServiceProviderKind::SystemdUser,
            "ssh_proxy",
            Some("tcp://127.0.0.1:19081".to_string()),
            Some("127.0.0.1:19080".parse().unwrap()),
            None,
            "$HOME/.ssh_proxy",
        );

        let result = run_lifecycle_commands(
            &executor,
            &spec,
            vec![LifecycleCommand::new(
                PeerLifecyclePhase::HealthProbe,
                "true",
            )],
        )
        .await
        .unwrap();

        assert_eq!(result.report.state, "healthy");
        assert_eq!(
            result.phase_reports.last().unwrap().phase,
            PeerLifecyclePhase::Healthy
        );
    }

    #[tokio::test]
    async fn lifecycle_workflow_fails_on_nonzero_command() {
        let executor = crate::peer_lifecycle::executor::FakeExecutor::default();
        executor.push_output(crate::ssh_client::ExecOutput {
            exit_status: 7,
            stdout: String::new(),
            stderr: "boom".to_string(),
        });
        let spec = crate::peer_lifecycle::spec::PeerLifecycleSpec::local_daemon(
            "local",
            "ssh_proxy",
            crate::peer_lifecycle::service_provider::ServiceProviderKind::SystemdUser,
            "ssh_proxy",
            None,
            None,
            None,
            "$HOME/.ssh_proxy",
        );

        let error = run_lifecycle_commands(
            &executor,
            &spec,
            vec![LifecycleCommand::new(
                PeerLifecyclePhase::InstallService,
                "false",
            )],
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("install_service"));
    }
}
