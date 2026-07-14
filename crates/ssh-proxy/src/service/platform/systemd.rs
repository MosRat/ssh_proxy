use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use ssh_proxy_platform::systemd::{
    SystemdDbusPlan, SystemdOperation, SystemdScope, run_systemd_plan,
};
use tracing::warn;

use crate::service::{
    inventory::{ServiceProbeState, ServiceProbeSummary},
    plan::{ServicePlan, ServiceScope, ensure_admin, platform_service_name},
};

use super::{
    command::{capture_command_output, run_command, run_command_output, write_text},
    probe::{contains_permission_denied, service_probe_summary},
};

pub(super) fn print(plan: &ServicePlan) -> Result<()> {
    let unit = unit(plan);
    println!(
        "{} systemd service:\n{}",
        match plan.scope {
            ServiceScope::User => "Linux user",
            ServiceScope::System => "Linux system",
        },
        unit
    );
    println!();
    if plan.scope == ServiceScope::User {
        println!("install: ssh_proxy service --scope user install");
        println!("status:  systemctl --user status ssh_proxy.service");
    } else {
        println!("install: ssh_proxy service --scope system install");
        println!("status:  systemctl status ssh_proxy.service");
    }
    Ok(())
}

pub(super) fn probe_summary(scope: ServiceScope) -> ServiceProbeSummary {
    let service_name = platform_service_name(scope);
    let args: Vec<&str> = match scope {
        ServiceScope::User => vec![
            "--user",
            "show",
            "--property=LoadState,ActiveState,UnitFileState",
            "ssh_proxy.service",
        ],
        ServiceScope::System => vec![
            "show",
            "--property=LoadState,ActiveState,UnitFileState",
            "ssh_proxy.service",
        ],
    };
    let capture = capture_command_output("systemctl", &args);
    let stdout = capture["stdout"].as_str().unwrap_or_default();
    let stderr = capture["stderr"].as_str().unwrap_or_default();
    let capture_ok = capture["ok"].as_bool().unwrap_or(false);
    let mut load_state = None;
    let mut active_state = None;
    let mut unit_file_state = None;
    for (key, value) in split_status_lines(stdout) {
        match key.as_str() {
            "LoadState" => load_state = Some(value),
            "ActiveState" => active_state = Some(value),
            "UnitFileState" => unit_file_state = Some(value),
            _ => {}
        }
    }
    let exists = load_state
        .as_deref()
        .is_some_and(|state| state != "not-found")
        || unit_file_state
            .as_deref()
            .is_some_and(|state| state != "not-found");
    let healthy = active_state.as_deref() == Some("active");
    let permission_denied = contains_permission_denied(stderr);
    let state = if healthy {
        ServiceProbeState::Healthy
    } else if exists {
        ServiceProbeState::Present
    } else if permission_denied {
        ServiceProbeState::PermissionDenied
    } else if capture_ok {
        ServiceProbeState::Missing
    } else {
        ServiceProbeState::Unknown
    };
    service_probe_summary(
        scope,
        service_name,
        state,
        exists,
        healthy,
        capture_ok || exists,
        permission_denied,
        json!({
            "program": "systemctl",
            "args": args,
            "capture": capture,
            "load_state": load_state,
            "active_state": active_state,
            "unit_file_state": unit_file_state,
        }),
    )
}

pub(super) fn install(plan: &ServicePlan) -> Result<()> {
    let path = unit_path(plan)?;
    write_text(&path, &unit(plan))?;
    let scope = dbus_scope(plan.scope);
    let unit = "ssh_proxy.service";
    if plan.scope == ServiceScope::User {
        run_dbus_or_command(
            SystemdDbusPlan::reload(scope),
            "systemctl",
            &["--user", "daemon-reload"],
        )?;
        run_command("loginctl", &["enable-linger", &current_user()])
            .map_err(|err| {
                eprintln!("warning: failed to enable systemd user linger: {err}");
                err
            })
            .ok();
        run_dbus_or_command(
            SystemdDbusPlan::unit(scope, SystemdOperation::Enable, unit),
            "systemctl",
            &["--user", "enable", "--now", "ssh_proxy.service"],
        )?;
        run_dbus_or_command(
            SystemdDbusPlan::unit(scope, SystemdOperation::Start, unit),
            "systemctl",
            &["--user", "start", "ssh_proxy.service"],
        )
    } else {
        run_dbus_or_command(
            SystemdDbusPlan::reload(scope),
            "systemctl",
            &["daemon-reload"],
        )?;
        run_dbus_or_command(
            SystemdDbusPlan::unit(scope, SystemdOperation::Enable, unit),
            "systemctl",
            &["enable", "--now", "ssh_proxy.service"],
        )?;
        run_dbus_or_command(
            SystemdDbusPlan::unit(scope, SystemdOperation::Start, unit),
            "systemctl",
            &["start", "ssh_proxy.service"],
        )
    }
}

pub(super) fn uninstall(plan: &ServicePlan) -> Result<()> {
    let scope = dbus_scope(plan.scope);
    if plan.scope == ServiceScope::User {
        run_dbus_or_command(
            SystemdDbusPlan::unit(scope, SystemdOperation::Disable, "ssh_proxy.service"),
            "systemctl",
            &["--user", "disable", "--now", "ssh_proxy.service"],
        )
        .ok();
    } else {
        run_dbus_or_command(
            SystemdDbusPlan::unit(scope, SystemdOperation::Disable, "ssh_proxy.service"),
            "systemctl",
            &["disable", "--now", "ssh_proxy.service"],
        )
        .ok();
    }
    let path = unit_path(plan)?;
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

pub(super) fn start(plan: &ServicePlan) -> Result<()> {
    let scope = dbus_scope(plan.scope);
    if plan.scope == ServiceScope::User {
        run_dbus_or_command(
            SystemdDbusPlan::unit(scope, SystemdOperation::Start, "ssh_proxy.service"),
            "systemctl",
            &["--user", "start", "ssh_proxy.service"],
        )
    } else {
        run_dbus_or_command(
            SystemdDbusPlan::unit(scope, SystemdOperation::Start, "ssh_proxy.service"),
            "systemctl",
            &["start", "ssh_proxy.service"],
        )
    }
}

pub(super) fn stop(plan: &ServicePlan) -> Result<()> {
    let scope = dbus_scope(plan.scope);
    if plan.scope == ServiceScope::User {
        run_dbus_or_command(
            SystemdDbusPlan::unit(scope, SystemdOperation::Stop, "ssh_proxy.service"),
            "systemctl",
            &["--user", "stop", "ssh_proxy.service"],
        )
    } else {
        run_dbus_or_command(
            SystemdDbusPlan::unit(scope, SystemdOperation::Stop, "ssh_proxy.service"),
            "systemctl",
            &["stop", "ssh_proxy.service"],
        )
    }
}

#[allow(dead_code)]
pub(super) fn status(plan: &ServicePlan) -> Result<()> {
    if plan.scope == ServiceScope::User {
        run_command_output(
            "systemctl",
            &["--user", "status", "--no-pager", "ssh_proxy.service"],
        )
    } else {
        run_command_output("systemctl", &["status", "--no-pager", "ssh_proxy.service"])
    }
}

pub(super) fn status_summary(plan: &ServicePlan) -> Value {
    let native = run_systemd_plan(&SystemdDbusPlan::unit(
        dbus_scope(plan.scope),
        SystemdOperation::Status,
        "ssh_proxy.service",
    ));
    if let Ok(outcome) = native {
        if outcome.ok {
            return outcome.to_json();
        }
    }
    if plan.scope == ServiceScope::User {
        capture_command_output(
            "systemctl",
            &["--user", "status", "--no-pager", "ssh_proxy.service"],
        )
    } else {
        capture_command_output("systemctl", &["status", "--no-pager", "ssh_proxy.service"])
    }
}

fn dbus_scope(scope: ServiceScope) -> SystemdScope {
    match scope {
        ServiceScope::User => SystemdScope::User,
        ServiceScope::System => SystemdScope::System,
    }
}

fn run_dbus_or_command(
    dbus_plan: SystemdDbusPlan,
    fallback_program: &str,
    fallback_args: &[&str],
) -> Result<()> {
    match run_systemd_plan(&dbus_plan) {
        Ok(outcome) if outcome.ok => Ok(()),
        Ok(outcome) => {
            warn!(
                execution_backend = "provider_command",
                fallback_used = true,
                provider = "systemd",
                native_backend = "systemd_dbus",
                status = outcome.status.as_deref().unwrap_or("operation"),
                fallback_program,
                "systemd D-Bus provider was not successful; falling back to provider command"
            );
            eprintln!(
                "warning: systemd D-Bus {} was not successful; falling back to {}",
                outcome.status.as_deref().unwrap_or("operation"),
                fallback_program
            );
            run_command(fallback_program, fallback_args)
        }
        Err(err) => {
            warn!(
                execution_backend = "provider_command",
                fallback_used = true,
                provider = "systemd",
                native_backend = "systemd_dbus",
                error = %err,
                fallback_program,
                "systemd D-Bus provider failed; falling back to provider command"
            );
            eprintln!(
                "warning: systemd D-Bus {} failed: {err}; falling back to {}",
                dbus_plan.method_name(),
                fallback_program
            );
            run_command(fallback_program, fallback_args)
        }
    }
}

fn split_status_lines(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (left, right) = line.split_once('=')?;
            Some((left.trim().to_string(), right.trim().to_string()))
        })
        .collect()
}

fn current_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| whoami::username().unwrap_or_else(|_| "unknown".to_string()))
}

fn unit(plan: &ServicePlan) -> String {
    format!(
        "[Unit]\nDescription=ssh_proxy local daemon\nAfter=network-online.target\nWants=network-online.target\nStartLimitIntervalSec=0\n\n[Service]\nExecStart={}\nRestart=always\nRestartSec=3\nKillSignal=SIGINT\n\n[Install]\nWantedBy=default.target\n",
        plan.daemon_command()
    )
}

fn unit_path(plan: &ServicePlan) -> Result<PathBuf> {
    if plan.scope == ServiceScope::User {
        let base = dirs::home_dir()
            .context("cannot determine home directory")?
            .join(".config/systemd/user");
        fs::create_dir_all(&base)
            .with_context(|| format!("failed to create {}", base.display()))?;
        Ok(base.join("ssh_proxy.service"))
    } else {
        ensure_admin("installing a system service requires root privileges")?;
        Ok(PathBuf::from("/etc/systemd/system/ssh_proxy.service"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::plan::ServicePlan;
    use crate::{cli, config};

    fn user_plan() -> ServicePlan {
        ServicePlan::new(
            cli::ServiceArgs {
                scope: cli::ServiceScope::User,
                control: None,
                transport: None,
                no_transport: false,
                token: None,
                tls_transport: None,
                quic_transport: None,
                tls_cert: None,
                tls_key: None,
                tls_client_ca: None,
                report_to: Vec::new(),
                install_dir: None,
                no_copy: true,
                json: false,
                elevate: false,
                command: cli::ServiceCommand::Print,
            },
            config::AppConfig::default(),
        )
        .expect("plan")
    }

    #[test]
    fn systemd_unit_keeps_foreground_daemon_command() {
        let plan = user_plan();
        let unit = unit(&plan);

        assert!(unit.contains("ExecStart="));
        assert!(unit.contains("daemon"));
        assert!(unit.contains("Restart=always"));
    }

    #[test]
    fn systemd_status_lines_are_parsed() {
        let parsed = split_status_lines("LoadState=loaded\nActiveState=active\n");

        assert_eq!(
            parsed,
            vec![
                ("LoadState".to_string(), "loaded".to_string()),
                ("ActiveState".to_string(), "active".to_string())
            ]
        );
    }
}
