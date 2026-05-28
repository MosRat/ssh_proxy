use serde::{Deserialize, Serialize};

use anyhow::{Context, Result, bail};

use super::{executor::PeerExecutor, report::PeerLifecycleReport, spec::PeerLifecycleSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LifecycleOperation {
    Install,
    Ensure,
    Start,
    Stop,
    Status,
    Repair,
    Rollback,
}

impl LifecycleOperation {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Ensure => "ensure",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Status => "status",
            Self::Repair => "repair",
            Self::Rollback => "rollback",
        }
    }
}

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
pub(crate) struct LifecycleCommandPlan {
    pub(crate) operation: LifecycleOperation,
    pub(crate) commands: Vec<LifecycleCommand>,
}

impl LifecycleCommandPlan {
    pub(crate) fn new(operation: LifecycleOperation) -> Self {
        Self {
            operation,
            commands: Vec::new(),
        }
    }

    pub(crate) fn push(mut self, command: LifecycleCommand) -> Self {
        self.commands.push(command);
        self
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LifecycleEvent {
    pub(crate) operation: LifecycleOperation,
    pub(crate) report: PeerLifecycleReport,
    pub(crate) message: String,
}

pub(crate) trait LifecycleEventSink {
    fn emit(&mut self, event: LifecycleEvent);
}

#[derive(Debug, Default)]
pub(crate) struct VecLifecycleEventSink {
    pub(crate) events: Vec<LifecycleEvent>,
}

impl LifecycleEventSink for VecLifecycleEventSink {
    fn emit(&mut self, event: LifecycleEvent) {
        self.events.push(event);
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PeerLifecycleWorkflowResult {
    pub(crate) operation: LifecycleOperation,
    pub(crate) report: PeerLifecycleReport,
    pub(crate) phase_reports: Vec<PeerLifecycleReport>,
}

pub(crate) async fn run_lifecycle_commands<E: PeerExecutor>(
    executor: &E,
    spec: &PeerLifecycleSpec,
    commands: Vec<LifecycleCommand>,
) -> Result<PeerLifecycleWorkflowResult> {
    run_lifecycle_plan(
        executor,
        spec,
        LifecycleCommandPlan {
            operation: LifecycleOperation::Ensure,
            commands,
        },
        &mut VecLifecycleEventSink::default(),
    )
    .await
}

pub(crate) async fn run_lifecycle_plan<E: PeerExecutor, S: LifecycleEventSink>(
    executor: &E,
    spec: &PeerLifecycleSpec,
    plan: LifecycleCommandPlan,
    sink: &mut S,
) -> Result<PeerLifecycleWorkflowResult> {
    let mut phase_reports = Vec::new();
    let prepare = phase_report_for_operation(spec, plan.operation, PeerLifecyclePhase::Prepare);
    sink.emit(LifecycleEvent {
        operation: plan.operation,
        report: prepare.clone(),
        message: "preparing lifecycle operation".to_string(),
    });
    phase_reports.push(prepare);
    for command in plan.commands {
        let mut report = phase_report_for_operation(spec, plan.operation, command.phase);
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
            sink.emit(LifecycleEvent {
                operation: plan.operation,
                report: report.clone(),
                message: format!("lifecycle phase {} failed", command.phase.as_str()),
            });
            phase_reports.push(report);
            bail!(
                "lifecycle phase {} failed with status {}: {}",
                command.phase.as_str(),
                output.exit_status,
                error
            );
        }
        sink.emit(LifecycleEvent {
            operation: plan.operation,
            report: report.clone(),
            message: format!("lifecycle phase {} completed", command.phase.as_str()),
        });
        phase_reports.push(report);
    }
    let report = phase_report_for_operation(spec, plan.operation, PeerLifecyclePhase::Healthy);
    sink.emit(LifecycleEvent {
        operation: plan.operation,
        report: report.clone(),
        message: "lifecycle operation completed".to_string(),
    });
    phase_reports.push(report.clone());
    Ok(PeerLifecycleWorkflowResult {
        operation: plan.operation,
        report,
        phase_reports,
    })
}

pub(crate) fn phase_report(
    spec: &PeerLifecycleSpec,
    phase: PeerLifecyclePhase,
) -> PeerLifecycleReport {
    phase_report_for_operation(spec, LifecycleOperation::Ensure, phase)
}

pub(crate) fn phase_report_for_operation(
    spec: &PeerLifecycleSpec,
    operation: LifecycleOperation,
    phase: PeerLifecyclePhase,
) -> PeerLifecycleReport {
    let mut report = PeerLifecycleReport::new(spec.target.clone(), phase);
    report.apply_spec(spec, operation);
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_phase_names_match_public_json_contract() {
        assert_eq!(PeerLifecyclePhase::Prepare.as_str(), "prepare");
        assert_eq!(LifecycleOperation::Install.as_str(), "install");
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
        assert_eq!(result.operation, LifecycleOperation::Ensure);
        assert_eq!(
            result.phase_reports.last().unwrap().phase,
            PeerLifecyclePhase::Healthy
        );
    }

    #[tokio::test]
    async fn lifecycle_plan_emits_operation_events() {
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
            None,
            None,
            None,
            "$HOME/.ssh_proxy",
        );
        let mut sink = VecLifecycleEventSink::default();

        let result = run_lifecycle_plan(
            &executor,
            &spec,
            LifecycleCommandPlan::new(LifecycleOperation::Install).push(LifecycleCommand::new(
                PeerLifecyclePhase::InstallService,
                "true",
            )),
            &mut sink,
        )
        .await
        .unwrap();

        assert_eq!(result.operation, LifecycleOperation::Install);
        assert_eq!(sink.events.len(), 3);
        assert_eq!(sink.events[0].operation, LifecycleOperation::Install);
        assert_eq!(sink.events[0].report.operation.as_deref(), Some("install"));
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
