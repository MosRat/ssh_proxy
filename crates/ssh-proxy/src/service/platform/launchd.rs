use std::{fmt::Write as _, fs, path::PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use ssh_proxy_core::external::ExternalActionClass;
use ssh_proxy_platform::{PlatformProbePlan, capture_command};

use crate::service::{
    inventory::{ServiceProbeState, ServiceProbeSummary},
    plan::{ServicePlan, ServiceScope, ensure_admin, platform_service_name},
};

use super::{
    command::{capture_command_output, run_command, run_command_output, write_text},
    probe::{contains_permission_denied, service_probe_summary},
};

const LAUNCHD_LABEL: &str = "local.ssh-proxy.daemon";

pub(super) fn print(plan: &ServicePlan) -> Result<()> {
    println!("macOS launchd plist:\n{}", plist(plan));
    println!();
    if plan.scope == ServiceScope::User {
        println!("install: ssh_proxy service --scope user install");
        println!("status:  launchctl print gui/$(id -u)/{LAUNCHD_LABEL}");
    } else {
        println!("install: ssh_proxy service --scope system install");
        println!("status:  launchctl print system/{LAUNCHD_LABEL}");
    }
    Ok(())
}

pub(super) fn probe_summary(scope: ServiceScope) -> ServiceProbeSummary {
    let service_name = platform_service_name(scope);
    let manifest_path = manifest_path(scope);
    let domain = match scope {
        ServiceScope::User => format!(
            "gui/{}/{}",
            current_uid().unwrap_or_else(|_| "0".to_string()),
            service_name
        ),
        ServiceScope::System => format!("system/{}", service_name),
    };
    let capture = capture_command_output("launchctl", &["print", &domain]);
    let stderr = capture["stderr"].as_str().unwrap_or_default();
    let plist_exists = manifest_path.as_ref().is_some_and(|path| path.exists());
    let loaded = capture["ok"].as_bool().unwrap_or(false);
    let permission_denied = contains_permission_denied(stderr);
    let exists = loaded || plist_exists;
    let healthy = loaded;
    let state = if healthy {
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
        healthy,
        exists || loaded,
        permission_denied,
        json!({
            "program": "launchctl",
            "domain": domain,
            "manifest_path": manifest_path.map(|path| path.display().to_string()),
            "capture": capture,
            "loaded": loaded,
            "plist_exists": plist_exists,
        }),
    )
}

pub(super) fn install(plan: &ServicePlan) -> Result<()> {
    let path = plist_path(plan)?;
    write_text(&path, &plist(plan))?;
    if plan.scope == ServiceScope::User {
        let target = format!("gui/{}", current_uid()?);
        run_command(
            "launchctl",
            &["bootstrap", &target, path.to_str().unwrap_or_default()],
        )?;
        run_command(
            "launchctl",
            &["enable", &format!("{target}/{LAUNCHD_LABEL}")],
        )
    } else {
        run_command(
            "launchctl",
            &["bootstrap", "system", path.to_str().unwrap_or_default()],
        )?;
        run_command("launchctl", &["enable", &format!("system/{LAUNCHD_LABEL}")])
    }
}

pub(super) fn uninstall(plan: &ServicePlan) -> Result<()> {
    let path = plist_path(plan)?;
    if plan.scope == ServiceScope::User {
        let target = format!("gui/{}", current_uid()?);
        run_command(
            "launchctl",
            &["bootout", &target, path.to_str().unwrap_or_default()],
        )
        .ok();
    } else {
        run_command(
            "launchctl",
            &["bootout", "system", path.to_str().unwrap_or_default()],
        )
        .ok();
    }
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

pub(super) fn start(plan: &ServicePlan) -> Result<()> {
    let domain = domain(plan)?;
    run_command(
        "launchctl",
        &["kickstart", "-k", &format!("{domain}/{LAUNCHD_LABEL}")],
    )
}

pub(super) fn stop(plan: &ServicePlan) -> Result<()> {
    let domain = domain(plan)?;
    run_command(
        "launchctl",
        &["kill", "TERM", &format!("{domain}/{LAUNCHD_LABEL}")],
    )
}

#[allow(dead_code)]
pub(super) fn status(plan: &ServicePlan) -> Result<()> {
    let domain = domain(plan)?;
    run_command_output(
        "launchctl",
        &["print", &format!("{domain}/{LAUNCHD_LABEL}")],
    )
}

pub(super) fn status_summary(plan: &ServicePlan) -> Value {
    match domain(plan) {
        Ok(domain) => capture_command_output(
            "launchctl",
            &["print", &format!("{domain}/{LAUNCHD_LABEL}")],
        ),
        Err(err) => json!({
            "ok": false,
            "program": "launchctl",
            "error": err.to_string(),
        }),
    }
}

fn domain(plan: &ServicePlan) -> Result<String> {
    if plan.scope == ServiceScope::User {
        Ok(format!("gui/{}", current_uid()?))
    } else {
        Ok("system".to_string())
    }
}

fn plist_path(plan: &ServicePlan) -> Result<PathBuf> {
    if plan.scope == ServiceScope::User {
        let base = dirs::home_dir()
            .context("cannot determine home directory")?
            .join("Library/LaunchAgents");
        fs::create_dir_all(&base)
            .with_context(|| format!("failed to create {}", base.display()))?;
        Ok(base.join(format!("{LAUNCHD_LABEL}.plist")))
    } else {
        ensure_admin("installing a system LaunchDaemon requires root privileges")?;
        Ok(PathBuf::from(format!(
            "/Library/LaunchDaemons/{LAUNCHD_LABEL}.plist"
        )))
    }
}

fn plist(plan: &ServicePlan) -> String {
    let mut program_arguments = String::new();
    for arg in plan.daemon_program_arguments() {
        let _ = writeln!(
            &mut program_arguments,
            "    <string>{}</string>",
            xml_escape(&arg)
        );
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{program_arguments}  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>3</integer>
  <key>StandardOutPath</key><string>/tmp/ssh_proxy.out.log</string>
  <key>StandardErrorPath</key><string>/tmp/ssh_proxy.err.log</string>
</dict>
</plist>"#
    )
}

fn manifest_path(scope: ServiceScope) -> Option<PathBuf> {
    match scope {
        ServiceScope::User => dirs::home_dir().map(|base| {
            base.join("Library")
                .join("LaunchAgents")
                .join(format!("{LAUNCHD_LABEL}.plist"))
        }),
        ServiceScope::System => Some(PathBuf::from(format!(
            "/Library/LaunchDaemons/{LAUNCHD_LABEL}.plist"
        ))),
    }
}

fn current_uid() -> Result<String> {
    let probe = PlatformProbePlan::new(
        "id",
        ["-u"],
        ExternalActionClass::RequiredProvider,
        "resolve current uid for launchd user domain",
        "launchd user domain requires the numeric uid",
    )
    .with_repair_action("install id/coreutils or use system service scope");
    let outcome = capture_command(probe.command_plan().clone())
        .context("failed to resolve current uid for launchd user domain")?;
    Ok(outcome.stdout.trim().to_string())
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::plan::ServicePlan;

    #[test]
    fn launchd_plist_tokenizes_daemon_command_without_shell() {
        let plan = ServicePlan::new(ServiceScope::User, false).expect("plan");
        let plist = plist(&plan);

        assert!(plist.contains("<key>ProgramArguments</key>"));
        assert!(plist.contains("<string>daemon</string>"));
        assert!(plist.contains("<string>serve</string>"));
        assert!(plist.contains("<key>KeepAlive</key><true/>"));
        assert!(plist.contains("local.ssh-proxy.daemon"));
        assert!(!plist.contains("<string>/bin/sh</string>"));
        assert!(!plist.contains("<string>-lc</string>"));
    }

    #[test]
    fn launchd_xml_escape_keeps_manifest_valid() {
        assert_eq!(xml_escape("a&b<c>d"), "a&amp;b&lt;c&gt;d");
    }
}
