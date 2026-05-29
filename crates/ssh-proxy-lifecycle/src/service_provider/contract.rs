use super::{
    kind::ServiceProviderKind,
    selection::{provider_dependency_classification, provider_dependency_state},
    status::{ProviderStatus, classify_provider_status},
};
use crate::{
    executor::ServiceControlAction,
    report::DependencyStatus,
    spec::PeerLifecycleSpec,
    workflow::{
        LifecycleAction, LifecycleOperation, LifecyclePlan, LifecycleStep, PeerLifecyclePhase,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceProviderPlan {
    pub kind: ServiceProviderKind,
    pub service_name: String,
    pub install_hint: String,
    pub status_hint: String,
}

impl ServiceProviderPlan {
    pub fn new(kind: ServiceProviderKind, service_name: impl Into<String>) -> Self {
        let service_name = service_name.into();
        let install_hint = match kind {
            ServiceProviderKind::WindowsScmSystem => {
                "windows-service via elevated install-worker".to_string()
            }
            ServiceProviderKind::WindowsScheduledTaskUser => {
                "schtasks /Create /SC ONLOGON".to_string()
            }
            ServiceProviderKind::SystemdUser => "systemctl --user enable --now".to_string(),
            ServiceProviderKind::SystemdSystem => "systemctl enable --now".to_string(),
            ServiceProviderKind::LaunchdUser => "launchctl bootstrap gui/<uid>".to_string(),
            ServiceProviderKind::LaunchdSystem => "launchctl bootstrap system".to_string(),
            ServiceProviderKind::NohupSupervisor => "managed nohup supervisor script".to_string(),
        };
        let status_hint = match kind {
            ServiceProviderKind::WindowsScmSystem => "sc.exe query".to_string(),
            ServiceProviderKind::WindowsScheduledTaskUser => "schtasks /Query".to_string(),
            ServiceProviderKind::SystemdUser => "systemctl --user status".to_string(),
            ServiceProviderKind::SystemdSystem => "systemctl status".to_string(),
            ServiceProviderKind::LaunchdUser => "launchctl print gui/<uid>".to_string(),
            ServiceProviderKind::LaunchdSystem => "launchctl print system".to_string(),
            ServiceProviderKind::NohupSupervisor => "pidfile plus child process probe".to_string(),
        };
        Self {
            kind,
            service_name,
            install_hint,
            status_hint,
        }
    }
}

pub trait PeerServiceProvider {
    fn kind(&self) -> ServiceProviderKind;
    fn service_name(&self) -> &str;
    fn dependency_report(&self) -> Vec<DependencyStatus>;
    fn lifecycle_plan(
        &self,
        spec: &PeerLifecycleSpec,
        operation: LifecycleOperation,
        command: Option<String>,
    ) -> LifecyclePlan;
    fn classify_status(&self, exit_status: u32, stdout: &str, stderr: &str) -> ProviderStatus;
    fn repair_hint(&self, blocker: &str) -> Option<String>;
}

impl PeerServiceProvider for ServiceProviderPlan {
    fn kind(&self) -> ServiceProviderKind {
        self.kind
    }

    fn service_name(&self) -> &str {
        &self.service_name
    }

    fn dependency_report(&self) -> Vec<DependencyStatus> {
        vec![
            DependencyStatus::new(
                format!("provider:{}", self.kind.manager_name()),
                provider_dependency_classification(self.kind),
                provider_dependency_state(self.kind),
            )
            .with_message(self.install_hint.clone()),
        ]
    }

    fn lifecycle_plan(
        &self,
        spec: &PeerLifecycleSpec,
        operation: LifecycleOperation,
        command: Option<String>,
    ) -> LifecyclePlan {
        let mut plan = LifecyclePlan::new(operation).push(LifecycleStep::new(
            PeerLifecyclePhase::DependencyCheck,
            LifecycleAction::Noop,
        ));
        let phase = lifecycle_phase_for_provider_operation(operation);
        let action = command
            .filter(|command| !command.trim().is_empty())
            .map(|command| LifecycleAction::RunCommand {
                command,
                stdin: None,
            })
            .unwrap_or_else(|| LifecycleAction::ServiceControl {
                service_name: spec.service_name.clone(),
                action: service_control_action(operation),
            });
        plan = plan.push(LifecycleStep::new(phase, action));
        plan
    }

    fn classify_status(&self, exit_status: u32, stdout: &str, stderr: &str) -> ProviderStatus {
        classify_provider_status(exit_status, stdout, stderr)
    }

    fn repair_hint(&self, blocker: &str) -> Option<String> {
        match blocker {
            "permission_denied" | "requires_elevation" if self.kind.requires_elevation() => {
                Some("retry with the interactive elevated repair action".to_string())
            }
            "service_missing" | "install_service_failed" => Some(format!(
                "reinstall {} using {}",
                self.service_name, self.install_hint
            )),
            "service_not_running" | "start_service_failed" => Some(format!(
                "start {} using {}",
                self.service_name, self.status_hint
            )),
            _ => None,
        }
    }
}

fn lifecycle_phase_for_provider_operation(operation: LifecycleOperation) -> PeerLifecyclePhase {
    match operation {
        LifecycleOperation::Install => PeerLifecyclePhase::InstallService,
        LifecycleOperation::Ensure | LifecycleOperation::Start | LifecycleOperation::Repair => {
            PeerLifecyclePhase::StartService
        }
        LifecycleOperation::Stop => PeerLifecyclePhase::Repairing,
        LifecycleOperation::Status => PeerLifecyclePhase::HealthProbe,
        LifecycleOperation::Rollback => PeerLifecyclePhase::Rollback,
    }
}

fn service_control_action(operation: LifecycleOperation) -> ServiceControlAction {
    match operation {
        LifecycleOperation::Install | LifecycleOperation::Ensure | LifecycleOperation::Repair => {
            ServiceControlAction::Install
        }
        LifecycleOperation::Start => ServiceControlAction::Start,
        LifecycleOperation::Stop => ServiceControlAction::Stop,
        LifecycleOperation::Status => ServiceControlAction::Status,
        LifecycleOperation::Rollback => ServiceControlAction::Rollback,
    }
}
