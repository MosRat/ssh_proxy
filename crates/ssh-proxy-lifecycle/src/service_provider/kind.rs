use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceProviderKind {
    WindowsScmSystem,
    WindowsScheduledTaskUser,
    SystemdUser,
    SystemdSystem,
    LaunchdUser,
    LaunchdSystem,
    NohupSupervisor,
}

impl ServiceProviderKind {
    pub fn from_manager_name(name: &str) -> Option<Self> {
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

    pub fn manager_name(self) -> &'static str {
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

    pub fn platform(self) -> &'static str {
        match self {
            Self::WindowsScmSystem | Self::WindowsScheduledTaskUser => "windows",
            Self::SystemdUser | Self::SystemdSystem | Self::NohupSupervisor => "linux",
            Self::LaunchdUser | Self::LaunchdSystem => "macos",
        }
    }

    pub fn persistent(self) -> bool {
        !matches!(self, Self::NohupSupervisor)
    }

    pub fn requires_elevation(self) -> bool {
        matches!(
            self,
            Self::WindowsScmSystem | Self::SystemdSystem | Self::LaunchdSystem
        )
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
            ServiceProviderKind::SystemdUser.manager_name(),
            "systemd_user"
        );
        assert!(ServiceProviderKind::WindowsScmSystem.requires_elevation());
        assert!(!ServiceProviderKind::WindowsScheduledTaskUser.requires_elevation());
    }
}
