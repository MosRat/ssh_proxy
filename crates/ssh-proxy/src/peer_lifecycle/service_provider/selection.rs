use crate::cli;
use ssh_proxy_core::model::{PersistenceMode, RemotePlatform};

use super::kind::ServiceProviderKind;

pub(crate) fn provider_for_remote_os(
    remote_os: cli::RemoteOs,
    persist: cli::PersistMode,
) -> ServiceProviderKind {
    provider_for_platform(remote_os.into(), persist.into())
}

pub(crate) fn provider_for_platform(
    remote_platform: RemotePlatform,
    persistence: PersistenceMode,
) -> ServiceProviderKind {
    match persistence {
        PersistenceMode::Systemd => ServiceProviderKind::SystemdUser,
        PersistenceMode::Nohup => ServiceProviderKind::NohupSupervisor,
        PersistenceMode::Launchd => ServiceProviderKind::LaunchdUser,
        PersistenceMode::Schtasks => ServiceProviderKind::WindowsScheduledTaskUser,
        PersistenceMode::None => ServiceProviderKind::NohupSupervisor,
        PersistenceMode::Auto => match remote_platform {
            RemotePlatform::Windows => ServiceProviderKind::WindowsScheduledTaskUser,
            RemotePlatform::Unix | RemotePlatform::Auto => ServiceProviderKind::SystemdUser,
        },
    }
}

pub(super) fn provider_dependency_classification(kind: ServiceProviderKind) -> &'static str {
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

pub(super) fn provider_dependency_state(kind: ServiceProviderKind) -> &'static str {
    match kind {
        ServiceProviderKind::NohupSupervisor => "fallback_provider",
        _ => "selected_provider",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_provider_defaults_match_production_order() {
        assert_eq!(
            provider_for_platform(RemotePlatform::Windows, PersistenceMode::Auto),
            ServiceProviderKind::WindowsScheduledTaskUser
        );
        assert_eq!(
            provider_for_platform(RemotePlatform::Auto, PersistenceMode::Auto),
            ServiceProviderKind::SystemdUser
        );
    }
}
