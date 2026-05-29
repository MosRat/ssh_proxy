use anyhow::{Context, Result, bail};

use crate::peer_lifecycle::{
    executor::PeerExecutor, report::PeerLifecycleReport, spec::PeerLifecycleSpec,
};

use super::{
    events::{LifecycleEvent, LifecycleEventSink, VecLifecycleEventSink},
    model::{
        LifecycleAction, LifecycleCommand, LifecycleCommandPlan, LifecycleOperation, LifecyclePlan,
        PeerLifecyclePhase,
    },
    outcome::{
        LifecycleActionResult, LifecycleFailure, LifecycleStepStatus, PeerLifecycleWorkflowResult,
    },
};

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
    let mut step_statuses = Vec::new();
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
            Ok(LifecycleActionResult::CommandOutput(output)) if output.exit_status != 0 => {
                let failure =
                    LifecycleFailure::from_command(step.phase, step.action.label(), &output);
                apply_failure_to_report(&mut report, &failure);
                emit_failure_event(sink, &mut events, plan.operation, &step.action, &report).await;
                phase_reports.push(report);
                step_statuses.push(LifecycleStepStatus::Failed(failure.clone()));
                bail!(
                    "lifecycle phase {} failed with status {}: {}",
                    failure.phase.as_str(),
                    failure.exit_status.unwrap_or(output.exit_status),
                    failure.message
                );
            }
            Ok(_) => {
                step_statuses.push(LifecycleStepStatus::Completed {
                    phase: step.phase,
                    action: step.action.label(),
                });
            }
            Err(err) => {
                let failure = LifecycleFailure::from_error(
                    step.phase,
                    step.action.label(),
                    format!("{err:#}"),
                );
                apply_failure_to_report(&mut report, &failure);
                emit_failure_event(sink, &mut events, plan.operation, &step.action, &report).await;
                phase_reports.push(report);
                step_statuses.push(LifecycleStepStatus::Failed(failure.clone()));
                bail!(
                    "lifecycle phase {} action {} failed: {}",
                    failure.phase.as_str(),
                    failure.action,
                    failure.message
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
        step_statuses,
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

async fn emit_failure_event<S: LifecycleEventSink>(
    sink: &mut S,
    events: &mut Vec<LifecycleEvent>,
    operation: LifecycleOperation,
    action: &LifecycleAction,
    report: &PeerLifecycleReport,
) {
    emit_event(
        sink,
        events,
        LifecycleEvent {
            operation,
            report: report.clone(),
            message: format!(
                "lifecycle phase {} action {} failed",
                report.phase.as_str(),
                action.label()
            ),
        },
    )
    .await;
}

fn apply_failure_to_report(report: &mut PeerLifecycleReport, failure: &LifecycleFailure) {
    report.state = PeerLifecyclePhase::Failed.as_str().to_string();
    report.phase = PeerLifecyclePhase::Failed;
    report.blocker = Some(failure.blocker.clone());
    report.last_error = Some(failure.message.clone());
}

async fn execute_action<E: PeerExecutor>(
    executor: &E,
    action: &LifecycleAction,
) -> Result<LifecycleActionResult> {
    match action {
        LifecycleAction::RunCommand { command, stdin } => Ok(LifecycleActionResult::CommandOutput(
            executor
                .exec_capture(command.clone(), stdin.clone())
                .await
                .with_context(|| format!("failed to run lifecycle command {command}"))?,
        )),
        LifecycleAction::StageBinary { source, target } => {
            executor
                .stage_binary(source.clone(), target.clone())
                .await?;
            Ok(LifecycleActionResult::Completed)
        }
        LifecycleAction::WriteArtifact {
            target,
            artifact,
            bytes,
        } => {
            executor
                .write_artifact(target.clone(), *artifact, bytes.clone())
                .await?;
            Ok(LifecycleActionResult::Completed)
        }
        LifecycleAction::ReadArtifact { target } => {
            executor.read_artifact(target.clone()).await?;
            Ok(LifecycleActionResult::Completed)
        }
        LifecycleAction::ProbeTcp { addr } => {
            executor.probe_tcp(*addr).await?;
            Ok(LifecycleActionResult::Completed)
        }
        LifecycleAction::ServiceControl {
            service_name,
            action,
        } => {
            let output = executor
                .service_control(service_name.clone(), *action)
                .await?;
            Ok(LifecycleActionResult::CommandOutput(output))
        }
        LifecycleAction::Noop => Ok(LifecycleActionResult::Completed),
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
