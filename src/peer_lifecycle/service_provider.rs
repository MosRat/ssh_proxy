use serde::{Deserialize, Serialize};

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

pub(crate) fn provider_for_remote_os(
    remote_os: crate::cli::RemoteOs,
    persist: crate::cli::PersistMode,
) -> ServiceProviderKind {
    match persist {
        crate::cli::PersistMode::Systemd => ServiceProviderKind::SystemdUser,
        crate::cli::PersistMode::Nohup => ServiceProviderKind::NohupSupervisor,
        crate::cli::PersistMode::Launchd => ServiceProviderKind::LaunchdUser,
        crate::cli::PersistMode::Schtasks => ServiceProviderKind::WindowsScheduledTaskUser,
        crate::cli::PersistMode::None => ServiceProviderKind::NohupSupervisor,
        crate::cli::PersistMode::Auto => match remote_os {
            crate::cli::RemoteOs::Windows => ServiceProviderKind::WindowsScheduledTaskUser,
            crate::cli::RemoteOs::Unix | crate::cli::RemoteOs::Auto => {
                ServiceProviderKind::SystemdUser
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_reports_stable_manager_names() {
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
    fn remote_provider_defaults_match_production_order() {
        assert_eq!(
            provider_for_remote_os(crate::cli::RemoteOs::Windows, crate::cli::PersistMode::Auto),
            ServiceProviderKind::WindowsScheduledTaskUser
        );
        assert_eq!(
            provider_for_remote_os(crate::cli::RemoteOs::Auto, crate::cli::PersistMode::Auto),
            ServiceProviderKind::SystemdUser
        );
    }
}
