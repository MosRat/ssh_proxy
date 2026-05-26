use std::process::Command;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

#[cfg(windows)]
use super::plan::command_quote;
use super::plan::{ServicePlan, ServiceScope, ensure_admin, platform_service_name};

#[cfg(windows)]
const SERVICE_NAME: &str = "ssh_proxy";
#[cfg(target_os = "macos")]
const LAUNCHD_LABEL: &str = "local.ssh-proxy.daemon";

fn run_command(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("failed to run {program}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("{program} exited with status {status}")
    }
}

#[allow(dead_code)]
fn run_command_output(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to run {program}"))?;
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    if output.status.success() {
        Ok(())
    } else {
        bail!("{program} exited with status {}", output.status)
    }
}

fn capture_command_output(program: &str, args: &[&str]) -> Value {
    match Command::new(program).args(args).output() {
        Ok(output) => json!({
            "ok": output.status.success(),
            "program": program,
            "args": args,
            "status": output.status.code(),
            "stdout": String::from_utf8_lossy(&output.stdout),
            "stderr": String::from_utf8_lossy(&output.stderr),
        }),
        Err(err) => json!({
            "ok": false,
            "program": program,
            "args": args,
            "error": err.to_string(),
        }),
    }
}

#[cfg(target_os = "linux")]
pub(super) fn platform_print(plan: &ServicePlan) -> Result<()> {
    let unit = linux_unit(plan);
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

#[cfg(target_os = "linux")]
pub(super) fn platform_install(plan: &ServicePlan) -> Result<()> {
    let path = linux_unit_path(plan)?;
    write_text(&path, &linux_unit(plan))?;
    if plan.scope == ServiceScope::User {
        run_command("systemctl", &["--user", "daemon-reload"])?;
        run_command("loginctl", &["enable-linger", &current_user()])
            .map_err(|err| {
                eprintln!("warning: failed to enable systemd user linger: {err}");
                err
            })
            .ok();
        run_command(
            "systemctl",
            &["--user", "enable", "--now", "ssh_proxy.service"],
        )
    } else {
        run_command("systemctl", &["daemon-reload"])?;
        run_command("systemctl", &["enable", "--now", "ssh_proxy.service"])
    }
}

#[cfg(target_os = "linux")]
fn current_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| whoami::username().unwrap_or_else(|_| "unknown".to_string()))
}

#[cfg(target_os = "linux")]
pub(super) fn platform_uninstall(plan: &ServicePlan) -> Result<()> {
    if plan.scope == ServiceScope::User {
        run_command(
            "systemctl",
            &["--user", "disable", "--now", "ssh_proxy.service"],
        )
        .ok();
    } else {
        run_command("systemctl", &["disable", "--now", "ssh_proxy.service"]).ok();
    }
    let path = linux_unit_path(plan)?;
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub(super) fn platform_start(plan: &ServicePlan) -> Result<()> {
    if plan.scope == ServiceScope::User {
        run_command("systemctl", &["--user", "start", "ssh_proxy.service"])
    } else {
        run_command("systemctl", &["start", "ssh_proxy.service"])
    }
}

#[cfg(target_os = "linux")]
pub(super) fn platform_stop(plan: &ServicePlan) -> Result<()> {
    if plan.scope == ServiceScope::User {
        run_command("systemctl", &["--user", "stop", "ssh_proxy.service"])
    } else {
        run_command("systemctl", &["stop", "ssh_proxy.service"])
    }
}

#[cfg(target_os = "linux")]
#[allow(dead_code)]
pub(super) fn platform_status(plan: &ServicePlan) -> Result<()> {
    if plan.scope == ServiceScope::User {
        run_command_output(
            "systemctl",
            &["--user", "status", "--no-pager", "ssh_proxy.service"],
        )
    } else {
        run_command_output("systemctl", &["status", "--no-pager", "ssh_proxy.service"])
    }
}

#[cfg(target_os = "linux")]
pub(super) fn platform_status_summary(plan: &ServicePlan) -> Value {
    if plan.scope == ServiceScope::User {
        capture_command_output(
            "systemctl",
            &["--user", "status", "--no-pager", "ssh_proxy.service"],
        )
    } else {
        capture_command_output("systemctl", &["status", "--no-pager", "ssh_proxy.service"])
    }
}

#[cfg(target_os = "linux")]
fn linux_unit(plan: &ServicePlan) -> String {
    format!(
        "[Unit]\nDescription=ssh_proxy local daemon\nAfter=network-online.target\nWants=network-online.target\nStartLimitIntervalSec=0\n\n[Service]\nExecStart={}\nRestart=always\nRestartSec=3\nKillSignal=SIGINT\n\n[Install]\nWantedBy=default.target\n",
        plan.daemon_command()
    )
}

#[cfg(target_os = "linux")]
fn linux_unit_path(plan: &ServicePlan) -> Result<PathBuf> {
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

#[cfg(target_os = "macos")]
pub(super) fn platform_print(plan: &ServicePlan) -> Result<()> {
    println!("macOS launchd plist:\n{}", launchd_plist(plan));
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

#[cfg(target_os = "macos")]
pub(super) fn platform_install(plan: &ServicePlan) -> Result<()> {
    let path = launchd_plist_path(plan)?;
    write_text(&path, &launchd_plist(plan))?;
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

#[cfg(target_os = "macos")]
pub(super) fn platform_uninstall(plan: &ServicePlan) -> Result<()> {
    let path = launchd_plist_path(plan)?;
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

#[cfg(target_os = "macos")]
pub(super) fn platform_start(plan: &ServicePlan) -> Result<()> {
    let domain = launchd_domain(plan)?;
    run_command(
        "launchctl",
        &["kickstart", "-k", &format!("{domain}/{LAUNCHD_LABEL}")],
    )
}

#[cfg(target_os = "macos")]
pub(super) fn platform_stop(plan: &ServicePlan) -> Result<()> {
    let domain = launchd_domain(plan)?;
    run_command(
        "launchctl",
        &["kill", "TERM", &format!("{domain}/{LAUNCHD_LABEL}")],
    )
}

#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub(super) fn platform_status(plan: &ServicePlan) -> Result<()> {
    let domain = launchd_domain(plan)?;
    run_command_output(
        "launchctl",
        &["print", &format!("{domain}/{LAUNCHD_LABEL}")],
    )
}

#[cfg(target_os = "macos")]
pub(super) fn platform_status_summary(plan: &ServicePlan) -> Value {
    match launchd_domain(plan) {
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

#[cfg(target_os = "macos")]
fn launchd_domain(plan: &ServicePlan) -> Result<String> {
    if plan.scope == ServiceScope::User {
        Ok(format!("gui/{}", current_uid()?))
    } else {
        Ok("system".to_string())
    }
}

#[cfg(target_os = "macos")]
fn launchd_plist_path(plan: &ServicePlan) -> Result<PathBuf> {
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

#[cfg(target_os = "macos")]
fn launchd_plist(plan: &ServicePlan) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/sh</string>
    <string>-lc</string>
    <string>{}</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>3</integer>
  <key>StandardOutPath</key><string>/tmp/ssh_proxy.out.log</string>
  <key>StandardErrorPath</key><string>/tmp/ssh_proxy.err.log</string>
</dict>
</plist>"#,
        xml_escape(&plan.daemon_command())
    )
}

#[cfg(target_os = "macos")]
fn current_uid() -> Result<String> {
    let output = Command::new("id").arg("-u").output()?;
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

#[cfg(target_os = "macos")]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(windows)]
pub(super) fn platform_print(plan: &ServicePlan) -> Result<()> {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => {
            println!("Windows user scheduled task:");
            println!("  {}", windows_schtasks_create(plan, &service_name));
            println!("  schtasks /Run /TN {service_name}");
            println!("  schtasks /Query /TN {service_name}");
        }
        ServiceScope::System => {
            println!("Windows system service:");
            println!("  {}", windows_sc_create(plan, &service_name));
            println!("  sc.exe start {service_name}");
            println!("  sc.exe query {service_name}");
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn platform_install(plan: &ServicePlan) -> Result<()> {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => run_command(
            "schtasks",
            &[
                "/Create",
                "/TN",
                &service_name,
                "/SC",
                "ONLOGON",
                "/RL",
                "LIMITED",
                "/F",
                "/TR",
                &plan.daemon_command(),
            ],
        ),
        ServiceScope::System => {
            ensure_admin("installing a Windows system service requires administrator privileges")?;
            run_command(
                "sc.exe",
                &[
                    "create",
                    &service_name,
                    "start=",
                    "auto",
                    "DisplayName=",
                    "ssh_proxy daemon",
                    "binPath=",
                    &plan.daemon_command(),
                ],
            )
        }
    }
}

#[cfg(windows)]
pub(super) fn platform_uninstall(plan: &ServicePlan) -> Result<()> {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => run_command("schtasks", &["/Delete", "/TN", &service_name, "/F"]),
        ServiceScope::System => {
            ensure_admin("removing a Windows system service requires administrator privileges")?;
            run_command("sc.exe", &["delete", &service_name])
        }
    }
}

#[cfg(windows)]
pub(super) fn platform_start(plan: &ServicePlan) -> Result<()> {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => run_command("schtasks", &["/Run", "/TN", &service_name]),
        ServiceScope::System => run_command("sc.exe", &["start", &service_name]),
    }
}

#[cfg(windows)]
pub(super) fn platform_stop(plan: &ServicePlan) -> Result<()> {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => run_command("schtasks", &["/End", "/TN", &service_name]),
        ServiceScope::System => run_command("sc.exe", &["stop", &service_name]),
    }
}

#[cfg(windows)]
#[allow(dead_code)]
pub(super) fn platform_status(plan: &ServicePlan) -> Result<()> {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => run_command_output("schtasks", &["/Query", "/TN", &service_name]),
        ServiceScope::System => run_command_output("sc.exe", &["query", &service_name]),
    }
}

#[cfg(windows)]
pub(super) fn platform_status_summary(plan: &ServicePlan) -> Value {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => capture_command_output("schtasks", &["/Query", "/TN", &service_name]),
        ServiceScope::System => capture_command_output("sc.exe", &["query", &service_name]),
    }
}

#[cfg(windows)]
fn windows_schtasks_create(plan: &ServicePlan, service_name: &str) -> String {
    format!(
        "schtasks /Create /TN {service_name} /SC ONLOGON /RL LIMITED /F /TR {}",
        command_quote(&plan.daemon_command())
    )
}

#[cfg(windows)]
fn windows_sc_create(plan: &ServicePlan, service_name: &str) -> String {
    format!(
        "sc.exe create {service_name} start= auto DisplayName= \"ssh_proxy daemon\" binPath= {}",
        command_quote(&plan.daemon_command())
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))
}
