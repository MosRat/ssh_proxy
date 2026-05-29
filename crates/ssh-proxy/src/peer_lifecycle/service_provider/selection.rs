use crate::cli;

use super::kind::ServiceProviderKind;

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
            provider_for_remote_os(cli::RemoteOs::Windows, cli::PersistMode::Auto),
            ServiceProviderKind::WindowsScheduledTaskUser
        );
        assert_eq!(
            provider_for_remote_os(cli::RemoteOs::Auto, cli::PersistMode::Auto),
            ServiceProviderKind::SystemdUser
        );
    }
}
