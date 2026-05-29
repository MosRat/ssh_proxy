use super::*;
use crate::peer_lifecycle::{artifacts::PeerArtifact, executor::ServiceControlAction};

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

#[test]
fn lifecycle_failure_classifies_command_output() {
    let output = crate::ssh_client::ExecOutput {
        exit_status: 42,
        stdout: String::new(),
        stderr: "denied".to_string(),
    };

    let failure = LifecycleFailure::from_command(
        PeerLifecyclePhase::InstallService,
        "service_control",
        &output,
    );

    assert_eq!(failure.blocker, "install_service_failed");
    assert_eq!(failure.exit_status, Some(42));
    assert_eq!(failure.message, "denied");
}
