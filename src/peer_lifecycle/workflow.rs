use std::{future::Future, net::SocketAddr, pin::Pin};

use serde::{Deserialize, Serialize};

use anyhow::{Context, Result, bail};

use super::{
    artifacts::PeerArtifact,
    executor::{PeerExecutor, ServiceControlAction},
    report::PeerLifecycleReport,
    spec::PeerLifecycleSpec,
};

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
pub(crate) enum LifecycleAction {
    RunCommand {
        command: String,
        stdin: Option<Vec<u8>>,
    },
    StageBinary {
        source: String,
        target: String,
    },
    WriteArtifact {
        target: String,
        artifact: PeerArtifact,
        bytes: Vec<u8>,
    },
    ReadArtifact {
        target: String,
    },
    ProbeTcp {
        addr: SocketAddr,
    },
    ServiceControl {
        service_name: String,
        action: ServiceControlAction,
    },
    Noop,
}

impl LifecycleAction {
    fn label(&self) -> &'static str {
        match self {
            Self::RunCommand { .. } => "run_command",
            Self::StageBinary { .. } => "stage_binary",
            Self::WriteArtifact { .. } => "write_artifact",
            Self::ReadArtifact { .. } => "read_artifact",
            Self::ProbeTcp { .. } => "probe_tcp",
            Self::ServiceControl { .. } => "service_control",
            Self::Noop => "noop",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LifecycleStep {
    pub(crate) phase: PeerLifecyclePhase,
    pub(crate) action: LifecycleAction,
}

impl LifecycleStep {
    pub(crate) fn new(phase: PeerLifecyclePhase, action: LifecycleAction) -> Self {
        Self { phase, action }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LifecyclePlan {
    pub(crate) operation: LifecycleOperation,
    pub(crate) steps: Vec<LifecycleStep>,
}

impl LifecyclePlan {
    pub(crate) fn new(operation: LifecycleOperation) -> Self {
        Self {
            operation,
            steps: Vec::new(),
        }
    }

    pub(crate) fn push(mut self, step: LifecycleStep) -> Self {
        self.steps.push(step);
        self
    }
}

impl From<LifecycleCommandPlan> for LifecyclePlan {
    fn from(plan: LifecycleCommandPlan) -> Self {
        let steps = plan
            .commands
            .into_iter()
            .map(|command| {
                LifecycleStep::new(
                    command.phase,
                    LifecycleAction::RunCommand {
                        command: command.command,
                        stdin: command.stdin,
                    },
                )
            })
            .collect();
        Self {
            operation: plan.operation,
            steps,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LifecycleEvent {
    pub(crate) operation: LifecycleOperation,
    pub(crate) report: PeerLifecycleReport,
    pub(crate) message: String,
}

pub(crate) type BoxEventFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

pub(crate) trait LifecycleEventSink {
    fn emit<'a>(&'a mut self, event: LifecycleEvent) -> BoxEventFuture<'a>;
}

#[derive(Debug, Default)]
pub(crate) struct VecLifecycleEventSink {
    pub(crate) events: Vec<LifecycleEvent>,
}

impl LifecycleEventSink for VecLifecycleEventSink {
    fn emit<'a>(&'a mut self, event: LifecycleEvent) -> BoxEventFuture<'a> {
        Box::pin(async move {
            self.events.push(event);
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PeerLifecycleWorkflowResult {
    pub(crate) operation: LifecycleOperation,
    pub(crate) report: PeerLifecycleReport,
    pub(crate) phase_reports: Vec<PeerLifecycleReport>,
    pub(crate) events: Vec<LifecycleEvent>,
    pub(crate) redacted_report: serde_json::Value,
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

pub(crate) async fn run_lifecycle_plan<E, S, P>(
    executor: &E,
    spec: &PeerLifecycleSpec,
    plan: P,
    sink: &mut S,
) -> Result<PeerLifecycleWorkflowResult>
where
    E: PeerExecutor,
    S: LifecycleEventSink,
    P: Into<LifecyclePlan>,
{
    let plan = plan.into();
    let mut phase_reports = Vec::new();
    let prepare = phase_report_for_operation(spec, plan.operation, PeerLifecyclePhase::Prepare);
    let mut events = Vec::new();
    emit_event(
        sink,
        &mut events,
        LifecycleEvent {
            operation: plan.operation,
            report: prepare.clone(),
            message: "preparing lifecycle operation".to_string(),
        },
    )
    .await;
    phase_reports.push(prepare);
    for step in plan.steps {
        let mut report = phase_report_for_operation(spec, plan.operation, step.phase);
        match execute_action(executor, &step.action).await {
            Ok(Some(output)) if output.exit_status != 0 => {
                let error = if output.stderr.trim().is_empty() {
                    output.stdout.trim().to_string()
                } else {
                    output.stderr.trim().to_string()
                };
                report.state = PeerLifecyclePhase::Failed.as_str().to_string();
                report.phase = PeerLifecyclePhase::Failed;
                report.blocker = Some(format!("{}_failed", step.phase.as_str()));
                report.last_error = Some(error.clone());
                emit_event(
                    sink,
                    &mut events,
                    LifecycleEvent {
                        operation: plan.operation,
                        report: report.clone(),
                        message: format!(
                            "lifecycle phase {} action {} failed",
                            step.phase.as_str(),
                            step.action.label()
                        ),
                    },
                )
                .await;
                phase_reports.push(report);
                bail!(
                    "lifecycle phase {} failed with status {}: {}",
                    step.phase.as_str(),
                    output.exit_status,
                    error
                );
            }
            Ok(_) => {}
            Err(err) => {
                let error = format!("{err:#}");
                report.state = PeerLifecyclePhase::Failed.as_str().to_string();
                report.phase = PeerLifecyclePhase::Failed;
                report.blocker = Some(format!("{}_failed", step.phase.as_str()));
                report.last_error = Some(error.clone());
                emit_event(
                    sink,
                    &mut events,
                    LifecycleEvent {
                        operation: plan.operation,
                        report: report.clone(),
                        message: format!(
                            "lifecycle phase {} action {} failed",
                            step.phase.as_str(),
                            step.action.label()
                        ),
                    },
                )
                .await;
                phase_reports.push(report);
                bail!(
                    "lifecycle phase {} action {} failed: {}",
                    step.phase.as_str(),
                    step.action.label(),
                    error
                );
            }
        }
        emit_event(
            sink,
            &mut events,
            LifecycleEvent {
                operation: plan.operation,
                report: report.clone(),
                message: format!("lifecycle phase {} completed", step.phase.as_str()),
            },
        )
        .await;
        phase_reports.push(report);
    }
    let report = phase_report_for_operation(spec, plan.operation, PeerLifecyclePhase::Healthy);
    emit_event(
        sink,
        &mut events,
        LifecycleEvent {
            operation: plan.operation,
            report: report.clone(),
            message: "lifecycle operation completed".to_string(),
        },
    )
    .await;
    phase_reports.push(report.clone());
    Ok(PeerLifecycleWorkflowResult {
        operation: plan.operation,
        redacted_report: report.to_redacted_value(),
        report,
        phase_reports,
        events,
    })
}

async fn emit_event<S: LifecycleEventSink>(
    sink: &mut S,
    events: &mut Vec<LifecycleEvent>,
    event: LifecycleEvent,
) {
    events.push(event.clone());
    sink.emit(event).await;
}

async fn execute_action<E: PeerExecutor>(
    executor: &E,
    action: &LifecycleAction,
) -> Result<Option<crate::ssh_client::ExecOutput>> {
    match action {
        LifecycleAction::RunCommand { command, stdin } => Ok(Some(
            executor
                .exec_capture(command.clone(), stdin.clone())
                .await
                .with_context(|| format!("failed to run lifecycle command {command}"))?,
        )),
        LifecycleAction::StageBinary { source, target } => {
            executor
                .stage_binary(source.clone(), target.clone())
                .await?;
            Ok(None)
        }
        LifecycleAction::WriteArtifact {
            target,
            artifact,
            bytes,
        } => {
            executor
                .write_artifact(target.clone(), *artifact, bytes.clone())
                .await?;
            Ok(None)
        }
        LifecycleAction::ReadArtifact { target } => {
            executor.read_artifact(target.clone()).await?;
            Ok(None)
        }
        LifecycleAction::ProbeTcp { addr } => {
            executor.probe_tcp(*addr).await?;
            Ok(None)
        }
        LifecycleAction::ServiceControl {
            service_name,
            action,
        } => Ok(Some(
            executor
                .service_control(service_name.clone(), *action)
                .await?,
        )),
        LifecycleAction::Noop => Ok(None),
    }
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
        assert_eq!(result.events.len(), 3);
        assert_eq!(result.redacted_report["operation"], "install");
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

    #[tokio::test]
    async fn lifecycle_action_plan_runs_structured_actions() {
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
            LifecyclePlan::new(LifecycleOperation::Install)
                .push(LifecycleStep::new(
                    PeerLifecyclePhase::StageBinary,
                    LifecycleAction::StageBinary {
                        source: "source".to_string(),
                        target: "target".to_string(),
                    },
                ))
                .push(LifecycleStep::new(
                    PeerLifecyclePhase::WriteConfig,
                    LifecycleAction::WriteArtifact {
                        target: "config.toml".to_string(),
                        artifact: PeerArtifact::Config,
                        bytes: b"config".to_vec(),
                    },
                ))
                .push(LifecycleStep::new(
                    PeerLifecyclePhase::StartService,
                    LifecycleAction::ServiceControl {
                        service_name: "ssh_proxy".to_string(),
                        action: ServiceControlAction::Start,
                    },
                )),
            &mut sink,
        )
        .await
        .unwrap();

        assert_eq!(result.report.state, "healthy");
        assert_eq!(
            executor.commands(),
            vec!["stage_binary source target".to_string()]
        );
        assert_eq!(executor.artifacts()[0].1, PeerArtifact::Config);
        assert_eq!(
            executor.service_controls(),
            vec![("ssh_proxy".to_string(), ServiceControlAction::Start)]
        );
    }
}
