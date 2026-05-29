use anyhow::Result;
use serde_json::Value;

use super::inventory::ServiceProbeSummary;
use super::plan::{ServicePlan, ServiceScope, platform_service_name};
#[cfg(target_os = "macos")]
mod launchd;

mod command;
mod probe;
#[cfg(target_os = "linux")]
mod systemd;
#[cfg(windows)]
mod windows_schtasks;
#[cfg(windows)]
mod windows_scm;

#[cfg(not(windows))]
pub(super) fn platform_install_requires_elevation(_plan: &ServicePlan) -> bool {
    false
}

#[cfg(not(windows))]
pub(super) fn platform_prepare_install(_plan: &ServicePlan) -> Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
pub(super) fn platform_print(plan: &ServicePlan) -> Result<()> {
    systemd::print(plan)
}

#[cfg(target_os = "linux")]
pub(super) fn platform_probe_summary(scope: ServiceScope) -> ServiceProbeSummary {
    systemd::probe_summary(scope)
}

#[cfg(target_os = "linux")]
pub(super) fn platform_install(plan: &ServicePlan) -> Result<()> {
    systemd::install(plan)
}

#[cfg(target_os = "linux")]
pub(super) fn platform_uninstall(plan: &ServicePlan) -> Result<()> {
    systemd::uninstall(plan)
}

#[cfg(target_os = "linux")]
pub(super) fn platform_start(plan: &ServicePlan) -> Result<()> {
    systemd::start(plan)
}

#[cfg(target_os = "linux")]
pub(super) fn platform_stop(plan: &ServicePlan) -> Result<()> {
    systemd::stop(plan)
}

#[cfg(target_os = "linux")]
#[allow(dead_code)]
pub(super) fn platform_status(plan: &ServicePlan) -> Result<()> {
    systemd::status(plan)
}

#[cfg(target_os = "linux")]
pub(super) fn platform_status_summary(plan: &ServicePlan) -> Value {
    systemd::status_summary(plan)
}

#[cfg(target_os = "macos")]
pub(super) fn platform_print(plan: &ServicePlan) -> Result<()> {
    launchd::print(plan)
}

#[cfg(target_os = "macos")]
pub(super) fn platform_probe_summary(scope: ServiceScope) -> ServiceProbeSummary {
    launchd::probe_summary(scope)
}

#[cfg(target_os = "macos")]
pub(super) fn platform_install(plan: &ServicePlan) -> Result<()> {
    launchd::install(plan)
}

#[cfg(target_os = "macos")]
pub(super) fn platform_uninstall(plan: &ServicePlan) -> Result<()> {
    launchd::uninstall(plan)
}

#[cfg(target_os = "macos")]
pub(super) fn platform_start(plan: &ServicePlan) -> Result<()> {
    launchd::start(plan)
}

#[cfg(target_os = "macos")]
pub(super) fn platform_stop(plan: &ServicePlan) -> Result<()> {
    launchd::stop(plan)
}

#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub(super) fn platform_status(plan: &ServicePlan) -> Result<()> {
    launchd::status(plan)
}

#[cfg(target_os = "macos")]
pub(super) fn platform_status_summary(plan: &ServicePlan) -> Value {
    launchd::status_summary(plan)
}

#[cfg(windows)]
pub(super) fn platform_print(plan: &ServicePlan) -> Result<()> {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => {
            windows_schtasks::print(plan, &service_name);
        }
        ServiceScope::System => {
            windows_scm::print(plan, &service_name);
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn platform_probe_summary(scope: ServiceScope) -> ServiceProbeSummary {
    let service_name = platform_service_name(scope);
    match scope {
        ServiceScope::User => windows_schtasks::probe_summary(scope, service_name),
        ServiceScope::System => windows_scm::probe_summary(scope, service_name),
    }
}

#[cfg(windows)]
pub(super) fn platform_install_requires_elevation(plan: &ServicePlan) -> bool {
    matches!(plan.scope, ServiceScope::System) && windows_scm::install_requires_elevation(plan)
}

#[cfg(windows)]
pub(super) fn platform_prepare_install(plan: &ServicePlan) -> Result<()> {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => windows_schtasks::prepare_install(&service_name),
        ServiceScope::System => windows_scm::prepare_install(&service_name),
    }
}

#[cfg(windows)]
pub(super) fn platform_install(plan: &ServicePlan) -> Result<()> {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => {
            windows_schtasks::install(plan, &service_name)?;
            platform_start(plan)
        }
        ServiceScope::System => windows_scm::install(plan, &service_name),
    }
}

#[cfg(windows)]
pub(super) fn platform_uninstall(plan: &ServicePlan) -> Result<()> {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => windows_schtasks::uninstall(&service_name),
        ServiceScope::System => windows_scm::uninstall(&service_name),
    }
}

#[cfg(windows)]
pub(super) fn platform_start(plan: &ServicePlan) -> Result<()> {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => windows_schtasks::start(&service_name),
        ServiceScope::System => windows_scm::start(plan, &service_name),
    }
}

#[cfg(windows)]
pub(super) fn platform_stop(plan: &ServicePlan) -> Result<()> {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => windows_schtasks::stop(&service_name),
        ServiceScope::System => windows_scm::stop(&service_name),
    }
}

#[cfg(windows)]
#[allow(dead_code)]
pub(super) fn platform_status(plan: &ServicePlan) -> Result<()> {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => windows_schtasks::status(&service_name),
        ServiceScope::System => windows_scm::status(&service_name),
    }
}

#[cfg(windows)]
pub(super) fn platform_status_summary(plan: &ServicePlan) -> Value {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => windows_schtasks::status_summary(&service_name),
        ServiceScope::System => windows_scm::status_summary(&service_name),
    }
}
