use std::{thread, time::Duration};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use ssh_proxy_platform::windows_tasks::{WindowsScheduledTaskPlan, register_logon_task};
use tracing::warn;

use crate::service::{
    inventory::{ServiceProbeState, ServiceProbeSummary},
    plan::{ServicePlan, ServiceScope, command_quote},
};

use super::{
    command::{capture_command_output, run_command, run_command_output},
    probe::{contains_permission_denied, service_probe_summary},
};

pub(super) fn print(plan: &ServicePlan, service_name: &str) {
    println!("Windows user scheduled task:");
    println!("  native: Task Scheduler COM RegisterTaskDefinition");
    println!("  {}", create_command(&plan.daemon_command(), service_name));
    println!("  schtasks /Run /TN {service_name}");
    println!("  schtasks /Query /TN {service_name}");
}

pub(super) fn probe_summary(scope: ServiceScope, service_name: String) -> ServiceProbeSummary {
    let capture = capture_command_output(
        "schtasks",
        &["/Query", "/TN", &service_name, "/FO", "LIST", "/V"],
    );
    let stdout = capture["stdout"].as_str().unwrap_or_default();
    let stderr = capture["stderr"].as_str().unwrap_or_default();
    let running = stdout.to_ascii_lowercase().contains("running");
    let exists = capture["ok"].as_bool().unwrap_or(false);
    let permission_denied = contains_permission_denied(stderr);
    let state = if running {
        ServiceProbeState::Healthy
    } else if exists {
        ServiceProbeState::Present
    } else if permission_denied {
        ServiceProbeState::PermissionDenied
    } else if capture["ok"].as_bool().unwrap_or(false) {
        ServiceProbeState::Missing
    } else {
        ServiceProbeState::Unknown
    };
    service_probe_summary(
        scope,
        service_name,
        state,
        exists,
        running,
        exists || running,
        permission_denied,
        json!({
            "program": "schtasks",
            "capture": capture,
            "running": running,
        }),
    )
}

pub(super) fn prepare_install(service_name: &str) -> Result<()> {
    if running(service_name) {
        run_command("schtasks", &["/End", "/TN", service_name])?;
        thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

pub(super) fn install(plan: &ServicePlan, service_name: &str) -> Result<()> {
    match install_native(plan, service_name) {
        Ok(()) => return Ok(()),
        Err(err) => {
            warn!(
                service_name,
                error = %err,
                "Task Scheduler COM install failed; falling back to schtasks"
            );
        }
    }
    run_command(
        "schtasks",
        &[
            "/Create",
            "/TN",
            service_name,
            "/SC",
            "ONLOGON",
            "/RL",
            "LIMITED",
            "/F",
            "/TR",
            &plan.daemon_command(),
        ],
    )
}

fn install_native(plan: &ServicePlan, service_name: &str) -> Result<()> {
    let args = plan.daemon_program_arguments();
    let (program, action_args) = args
        .split_first()
        .context("daemon program arguments cannot be empty")?;
    register_logon_task(&WindowsScheduledTaskPlan::new(
        service_name,
        program,
        action_args.iter().cloned(),
    ))?;
    Ok(())
}

pub(super) fn uninstall(service_name: &str) -> Result<()> {
    run_command("schtasks", &["/Delete", "/TN", service_name, "/F"])
}

pub(super) fn start(service_name: &str) -> Result<()> {
    if running(service_name) {
        return Ok(());
    }
    run_command("schtasks", &["/Run", "/TN", service_name])
}

pub(super) fn stop(service_name: &str) -> Result<()> {
    run_command("schtasks", &["/End", "/TN", service_name])
}

pub(super) fn status(service_name: &str) -> Result<()> {
    run_command_output("schtasks", &["/Query", "/TN", service_name])
}

pub(super) fn status_summary(service_name: &str) -> Value {
    capture_command_output("schtasks", &["/Query", "/TN", service_name])
}

pub(super) fn running(service_name: &str) -> bool {
    capture_command_output(
        "schtasks",
        &["/Query", "/TN", service_name, "/FO", "LIST", "/V"],
    )["stdout"]
        .as_str()
        .map(|stdout| stdout.to_ascii_lowercase().contains("running"))
        .unwrap_or(false)
}

fn create_command(daemon_command: &str, service_name: &str) -> String {
    format!(
        "schtasks /Create /TN {service_name} /SC ONLOGON /RL LIMITED /F /TR {}",
        command_quote(daemon_command)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schtasks_create_command_quotes_daemon_command() {
        let command = create_command("ssh_proxy daemon serve", "ssh_proxy_user");

        assert!(command.contains("schtasks /Create"));
        assert!(command.contains("/SC ONLOGON"));
        assert!(command.contains("/TR"));
    }
}
