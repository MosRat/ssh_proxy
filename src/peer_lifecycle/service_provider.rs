use serde::{Deserialize, Serialize};

use crate::cli;

use super::{
    commands,
    executor::ServiceControlAction,
    report::DependencyStatus,
    spec::PeerLifecycleSpec,
    workflow::{
        LifecycleAction, LifecycleOperation, LifecyclePlan, LifecycleStep, PeerLifecyclePhase,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ServiceProviderKind {
    WindowsScmSystem,
    WindowsScheduledTaskUser,
    SystemdUser,
    SystemdSystem,
    LaunchdUser,
    LaunchdSystem,
    NohupSupervisor,
}

impl ServiceProviderKind {
    pub(crate) fn from_manager_name(name: &str) -> Option<Self> {
        match name {
            "windows_scm_system" => Some(Self::WindowsScmSystem),
            "windows_schtasks_user" => Some(Self::WindowsScheduledTaskUser),
            "systemd_user" => Some(Self::SystemdUser),
            "systemd_system" => Some(Self::SystemdSystem),
            "launchd_user" => Some(Self::LaunchdUser),
            "launchd_system" => Some(Self::LaunchdSystem),
            "nohup_supervisor" => Some(Self::NohupSupervisor),
            _ => None,
        }
    }

    pub(crate) fn manager_name(self) -> &'static str {
        match self {
            Self::WindowsScmSystem => "windows_scm_system",
            Self::WindowsScheduledTaskUser => "windows_schtasks_user",
            Self::SystemdUser => "systemd_user",
            Self::SystemdSystem => "systemd_system",
            Self::LaunchdUser => "launchd_user",
            Self::LaunchdSystem => "launchd_system",
            Self::NohupSupervisor => "nohup_supervisor",
        }
    }

    pub(crate) fn platform(self) -> &'static str {
        match self {
            Self::WindowsScmSystem | Self::WindowsScheduledTaskUser => "windows",
            Self::SystemdUser | Self::SystemdSystem | Self::NohupSupervisor => "linux",
            Self::LaunchdUser | Self::LaunchdSystem => "macos",
        }
    }

    pub(crate) fn persistent(self) -> bool {
        !matches!(self, Self::NohupSupervisor)
    }

    pub(crate) fn requires_elevation(self) -> bool {
        matches!(
            self,
            Self::WindowsScmSystem | Self::SystemdSystem | Self::LaunchdSystem
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServiceProviderPlan {
    pub(crate) kind: ServiceProviderKind,
    pub(crate) service_name: String,
    pub(crate) install_hint: String,
    pub(crate) status_hint: String,
}

impl ServiceProviderPlan {
    pub(crate) fn new(kind: ServiceProviderKind, service_name: impl Into<String>) -> Self {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderStatusState {
    Healthy,
    Present,
    Missing,
    PermissionDenied,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProviderStatus {
    pub(crate) state: ProviderStatusState,
    pub(crate) healthy: bool,
    pub(crate) message: String,
}

pub(crate) trait PeerServiceProvider {
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
        let text = format!("{stdout}\n{stderr}").to_ascii_lowercase();
        let permission_denied = text.contains("access is denied")
            || text.contains("permission denied")
            || text.contains("not permitted")
            || text.contains("operation not permitted");
        let missing = text.contains("not-found")
            || text.contains("not found")
            || text.contains("could not be found")
            || text.contains("does not exist");
        let healthy = exit_status == 0
            && (text.contains("running")
                || text.contains("active")
                || text.contains("healthy")
                || text.contains("success"));
        let state = if permission_denied {
            ProviderStatusState::PermissionDenied
        } else if healthy {
            ProviderStatusState::Healthy
        } else if missing {
            ProviderStatusState::Missing
        } else if exit_status == 0 {
            ProviderStatusState::Present
        } else {
            ProviderStatusState::Unknown
        };
        ProviderStatus {
            state,
            healthy,
            message: if stderr.trim().is_empty() {
                stdout.trim().to_string()
            } else {
                stderr.trim().to_string()
            },
        }
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

pub(crate) fn provider_for_remote_os(
    remote_os: cli::RemoteOs,
    persist: cli::PersistMode,
) -> ServiceProviderKind {
    match persist {
        cli::PersistMode::Systemd => ServiceProviderKind::SystemdUser,
        cli::PersistMode::Nohup => ServiceProviderKind::NohupSupervisor,
        cli::PersistMode::Launchd => ServiceProviderKind::LaunchdUser,
        cli::PersistMode::Schtasks => ServiceProviderKind::WindowsScheduledTaskUser,
        cli::PersistMode::None => ServiceProviderKind::NohupSupervisor,
        cli::PersistMode::Auto => match remote_os {
            cli::RemoteOs::Windows => ServiceProviderKind::WindowsScheduledTaskUser,
            cli::RemoteOs::Unix | cli::RemoteOs::Auto => ServiceProviderKind::SystemdUser,
        },
    }
}

fn provider_dependency_classification(kind: ServiceProviderKind) -> &'static str {
    match kind {
        ServiceProviderKind::WindowsScmSystem
        | ServiceProviderKind::SystemdSystem
        | ServiceProviderKind::LaunchdSystem => "required",
        ServiceProviderKind::WindowsScheduledTaskUser
        | ServiceProviderKind::SystemdUser
        | ServiceProviderKind::LaunchdUser => "required",
        ServiceProviderKind::NohupSupervisor => "emergency_compat",
    }
}

fn provider_dependency_state(kind: ServiceProviderKind) -> &'static str {
    match kind {
        ServiceProviderKind::NohupSupervisor => "fallback_provider",
        _ => "selected_provider",
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

pub(crate) fn provider_for_remote_report(
    service_manager: &str,
    remote_os: cli::RemoteOs,
    persist: cli::PersistMode,
) -> ServiceProviderKind {
    ServiceProviderKind::from_manager_name(service_manager)
        .unwrap_or_else(|| provider_for_remote_os(remote_os, persist))
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteServiceInstallPlan {
    pub(crate) provider: ServiceProviderPlan,
    pub(crate) command: String,
    pub(crate) reported_service_manager: String,
}

pub(crate) fn remote_service_install_plan(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> RemoteServiceInstallPlan {
    let kind = provider_for_remote_os(args.remote_os, args.persist);
    let command = match args.persist {
        cli::PersistMode::None => String::new(),
        cli::PersistMode::Auto => {
            if args.remote_os == cli::RemoteOs::Windows {
                commands::remote_schtasks_install_command(remote_path, args)
            } else {
                commands::remote_auto_install_command(remote_path, args)
            }
        }
        cli::PersistMode::Systemd => commands::remote_systemd_install_command(remote_path, args),
        cli::PersistMode::Nohup => commands::remote_nohup_start_command(remote_path, args, true),
        cli::PersistMode::Launchd => commands::remote_launchd_install_command(remote_path, args),
        cli::PersistMode::Schtasks => commands::remote_schtasks_install_command(remote_path, args),
    };
    let reported_service_manager = match args.persist {
        cli::PersistMode::None => "none",
        cli::PersistMode::Auto if args.remote_os == cli::RemoteOs::Windows => {
            ServiceProviderKind::WindowsScheduledTaskUser.manager_name()
        }
        cli::PersistMode::Auto => "auto",
        _ => kind.manager_name(),
    }
    .to_string();
    RemoteServiceInstallPlan {
        provider: ServiceProviderPlan::new(kind, "ssh-proxy-helper"),
        command,
        reported_service_manager,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_reports_stable_manager_names() {
        assert_eq!(
            ServiceProviderKind::from_manager_name("windows_scm_system"),
            Some(ServiceProviderKind::WindowsScmSystem)
        );
        assert_eq!(ServiceProviderKind::from_manager_name("auto"), None);
        assert_eq!(
            ServiceProviderKind::WindowsScmSystem.manager_name(),
            "windows_scm_system"
        );
        assert_eq!(
            ServiceProviderKind::SystemdUser.manager_name(),
            "systemd_user"
        );
        assert!(ServiceProviderKind::WindowsScmSystem.requires_elevation());
        assert!(!ServiceProviderKind::WindowsScheduledTaskUser.requires_elevation());
    }

    #[test]
    fn provider_contract_builds_lifecycle_plan() {
        let provider = ServiceProviderPlan::new(ServiceProviderKind::SystemdUser, "ssh_proxy");
        let args = cli::InstallRemoteArgs {
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
        };
        let spec = PeerLifecycleSpec::remote_peer(
            "edge",
            "/home/me/bin/ssh_proxy",
            &args,
            ServiceProviderKind::SystemdUser,
        );

        let plan = provider.lifecycle_plan(
            &spec,
            LifecycleOperation::Install,
            Some("systemctl --user restart ssh_proxy".to_string()),
        );

        assert_eq!(provider.kind(), ServiceProviderKind::SystemdUser);
        assert_eq!(provider.service_name(), "ssh_proxy");
        assert_eq!(provider.dependency_report()[0].state, "selected_provider");
        assert_eq!(plan.operation, LifecycleOperation::Install);
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].phase, PeerLifecyclePhase::DependencyCheck);
        assert_eq!(plan.steps[1].phase, PeerLifecyclePhase::InstallService);
    }

    #[test]
    fn provider_status_classification_is_stable() {
        let provider = ServiceProviderPlan::new(ServiceProviderKind::SystemdUser, "ssh_proxy");

        let healthy = provider.classify_status(0, "ActiveState=active", "");
        let missing = provider.classify_status(3, "LoadState=not-found", "");
        let denied = provider.classify_status(1, "", "permission denied");

        assert_eq!(healthy.state, ProviderStatusState::Healthy);
        assert_eq!(missing.state, ProviderStatusState::Missing);
        assert_eq!(denied.state, ProviderStatusState::PermissionDenied);
        assert!(provider.repair_hint("service_missing").is_some());
    }

    #[test]
    fn remote_provider_defaults_match_production_order() {
        assert_eq!(
            provider_for_remote_os(cli::RemoteOs::Windows, cli::PersistMode::Auto),
            ServiceProviderKind::WindowsScheduledTaskUser
        );
        assert_eq!(
            provider_for_remote_os(cli::RemoteOs::Auto, cli::PersistMode::Auto),
            ServiceProviderKind::SystemdUser
        );
    }

    #[test]
    fn remote_install_plan_preserves_auto_reporting_contract() {
        let args = cli::InstallRemoteArgs {
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
            persist: cli::PersistMode::Auto,
        };

        let plan = remote_service_install_plan("/home/me/bin/ssh_proxy", &args);

        assert_eq!(plan.provider.kind, ServiceProviderKind::SystemdUser);
        assert_eq!(plan.reported_service_manager, "auto");
        assert!(plan.command.contains("systemctl --user"));
        assert!(plan.command.contains("nohup"));
    }
}
