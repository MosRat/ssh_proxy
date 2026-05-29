#[cfg(any(windows, target_os = "macos"))]
use std::process::Command;

#[cfg(windows)]
use std::{
    ffi::{OsStr, OsString},
    os::windows::ffi::OsStrExt,
    path::Path,
    thread,
    time::Duration,
};
#[cfg(windows)]
use windows_service::{
    Error as WindowsServiceError,
    service::{
        Service, ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState,
        ServiceType,
    },
    service_manager::{ServiceManager, ServiceManagerAccess},
};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::{fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError},
    System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
    UI::{
        Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::SW_HIDE,
    },
};

use super::inventory::{ServiceProbeState, ServiceProbeSummary};
#[cfg(windows)]
use super::plan::command_quote;
use super::plan::{ServicePlan, ServiceScope, ensure_admin, platform_service_name};
#[cfg(windows)]
use crate::install_report;
#[cfg(target_os = "macos")]
const LAUNCHD_LABEL: &str = "local.ssh-proxy.daemon";

mod command;
mod probe;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use command::write_text;
use command::{capture_command_output, run_command, run_command_output};
use probe::{contains_permission_denied, service_probe_summary};

#[cfg(not(windows))]
pub(super) fn platform_install_requires_elevation(_plan: &ServicePlan) -> bool {
    false
}

#[cfg(not(windows))]
pub(super) fn platform_prepare_install(_plan: &ServicePlan) -> Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn split_status_lines(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (left, right) = line.split_once('=')?;
            Some((left.trim().to_string(), right.trim().to_string()))
        })
        .collect()
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
pub(super) fn platform_probe_summary(scope: ServiceScope) -> ServiceProbeSummary {
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
pub(super) fn platform_probe_summary(scope: ServiceScope) -> ServiceProbeSummary {
    let service_name = platform_service_name(scope);
    let manifest_path = macos_manifest_path(scope);
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
fn macos_manifest_path(scope: ServiceScope) -> Option<PathBuf> {
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
pub(super) fn platform_probe_summary(scope: ServiceScope) -> ServiceProbeSummary {
    let service_name = platform_service_name(scope);
    match scope {
        ServiceScope::User => {
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
        ServiceScope::System => {
            let capture = capture_command_output("sc.exe", &["query", &service_name]);
            let stdout = capture["stdout"].as_str().unwrap_or_default();
            let stderr = capture["stderr"].as_str().unwrap_or_default();
            let running = stdout.to_ascii_uppercase().contains("RUNNING");
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
                    "program": "sc.exe",
                    "capture": capture,
                    "running": running,
                }),
            )
        }
    }
}

#[cfg(windows)]
pub(super) fn platform_install_requires_elevation(plan: &ServicePlan) -> bool {
    matches!(plan.scope, ServiceScope::System) && plan.elevate && !is_elevated_for_platform()
}

#[cfg(windows)]
pub(super) fn platform_prepare_install(plan: &ServicePlan) -> Result<()> {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => {
            if windows_task_running(&service_name) {
                run_command("schtasks", &["/End", "/TN", &service_name])?;
                thread::sleep(Duration::from_millis(500));
            }
            Ok(())
        }
        ServiceScope::System => {
            ensure_admin(
                "preparing a Windows system service install requires administrator privileges",
            )?;
            windows_stop_service_for_replace(&service_name)
        }
    }
}

#[cfg(windows)]
pub(super) fn platform_install(plan: &ServicePlan) -> Result<()> {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => {
            run_command(
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
            )?;
            platform_start(plan)
        }
        ServiceScope::System => {
            if plan.elevate && !is_elevated_for_platform() {
                return platform_install_elevated(plan);
            }
            ensure_admin("installing a Windows system service requires administrator privileges")?;
            let manager = windows_service_manager(
                ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
            )?;
            let service_info = windows_system_service_info(plan, &service_name);
            let access = ServiceAccess::CHANGE_CONFIG
                | ServiceAccess::QUERY_STATUS
                | ServiceAccess::START
                | ServiceAccess::STOP;
            let service = match manager.open_service(&service_name, access) {
                Ok(service) => {
                    service.change_config(&service_info).with_context(|| {
                        format!("failed to configure Windows service {service_name}")
                    })?;
                    service
                }
                Err(err) if windows_service_error_code(&err) == Some(1060) => manager
                    .create_service(&service_info, access)
                    .with_context(|| format!("failed to create Windows service {service_name}"))?,
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("failed to open Windows service {service_name}"));
                }
            };
            if let Err(err) = service.set_description("ssh_proxy local daemon control plane") {
                eprintln!("warning: failed to set Windows service description: {err}");
            }
            if windows_service_status_is(&service, ServiceState::Running) {
                Ok(())
            } else {
                platform_start(plan)
            }
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
            if let Some(service) = windows_open_system_service(
                &service_name,
                ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS | ServiceAccess::STOP,
            )? {
                if !windows_service_status_is(&service, ServiceState::Stopped) {
                    let _ = service.stop();
                    windows_wait_service_stopped(&service_name)?;
                }
                service
                    .delete()
                    .with_context(|| format!("failed to delete Windows service {service_name}"))?;
            }
            Ok(())
        }
    }
}

#[cfg(windows)]
pub(super) fn platform_install_elevated(plan: &ServicePlan) -> Result<()> {
    let log_path = install_report::install_log_path_for_pid(std::process::id());
    let mut service_args = vec![
        "daemon-install-worker".to_string(),
        "--scope".to_string(),
        "system".to_string(),
        "--json".to_string(),
        "--install-log".to_string(),
        log_path.display().to_string(),
    ];
    if !plan.copy_exe {
        service_args.push("--no-copy".to_string());
    }
    let exit_code = match run_elevated_process(&plan.source_exe, &service_args) {
        Ok(code) => code,
        Err(native_err) => {
            let fallback = run_powershell_elevated(&plan.source_exe, &service_args)?;
            if fallback != 0 {
                return elevated_install_failed(&log_path, fallback, Some(native_err.to_string()));
            }
            fallback
        }
    };
    if exit_code == 0 {
        return Ok(());
    }
    elevated_install_failed(&log_path, exit_code, None)
}

#[cfg(windows)]
fn run_elevated_process(exe: &Path, args: &[String]) -> Result<u32> {
    let verb = wide("runas");
    let file = wide_os(exe.as_os_str());
    let parameters = wide(&join_windows_args(args));
    let mut info = SHELLEXECUTEINFOW::default();
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.nShow = SW_HIDE;
    let launched = unsafe { ShellExecuteExW(&mut info) };
    if launched == 0 {
        let code = unsafe { GetLastError() };
        if code == 1223 {
            return Ok(1223);
        }
        bail!("ShellExecuteW runas failed with Windows error {code}");
    }
    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut exit_code = 1_u32;
        if GetExitCodeProcess(info.hProcess, &mut exit_code) == 0 {
            let code = GetLastError();
            CloseHandle(info.hProcess);
            bail!("GetExitCodeProcess failed with Windows error {code}");
        }
        CloseHandle(info.hProcess);
        Ok(exit_code)
    }
}

#[cfg(windows)]
fn run_powershell_elevated(exe: &Path, service_args: &[String]) -> Result<u32> {
    let elevated_args = vec![
        "-NoProfile".to_string(),
        "-ExecutionPolicy".to_string(),
        "Bypass".to_string(),
        "-Command".to_string(),
        format!(
            "& {} {}; $code = $LASTEXITCODE; if ($null -eq $code) {{ $code = 0 }}; exit $code",
            powershell_quote(&exe.display().to_string()),
            powershell_array(service_args),
        ),
    ];
    let command = format!(
        "$p = Start-Process -FilePath 'powershell.exe' -ArgumentList {} -Verb RunAs -WindowStyle Hidden -Wait -PassThru; if ($null -eq $p.ExitCode) {{ exit 1223 }}; exit $p.ExitCode",
        powershell_quote(&join_windows_args(&elevated_args)),
    );
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &command,
        ])
        .output()
        .context("failed to run elevated PowerShell installer fallback")?;
    Ok(output.status.code().unwrap_or(1) as u32)
}

#[cfg(windows)]
fn elevated_install_failed(
    log_path: &Path,
    exit_code: u32,
    native_error: Option<String>,
) -> Result<()> {
    if exit_code == 1223 {
        let install_id = format!("install-{}-cancelled", std::process::id());
        let _ = install_report::append_install_event(
            log_path,
            &install_id,
            "cancelled",
            "cancelled_by_user",
            "elevated daemon install was cancelled by the user",
            Some("cancelled_by_user"),
        );
        let report = install_report::install_report_from_log(log_path);
        bail!(
            "ssh_proxy daemon install cancelled_by_user; elevated installer report: {}",
            serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_string())
        );
    }
    let report = install_report::install_report_from_log(log_path);
    bail!(
        "ssh_proxy daemon install failed with code {exit_code}; elevated installer report: {}{}",
        serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_string()),
        native_error
            .map(|error| format!("; native launcher error: {error}"))
            .unwrap_or_default()
    )
}

#[cfg(windows)]
pub(super) fn platform_start(plan: &ServicePlan) -> Result<()> {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => {
            if windows_task_running(&service_name) {
                return Ok(());
            }
            run_command("schtasks", &["/Run", "/TN", &service_name])
        }
        ServiceScope::System => {
            if plan.elevate && !is_elevated_for_platform() {
                return platform_install_elevated(plan);
            }
            if windows_service_running(&service_name) {
                return Ok(());
            }
            let service = windows_open_system_service(
                &service_name,
                ServiceAccess::START | ServiceAccess::QUERY_STATUS,
            )?
            .ok_or_else(|| anyhow::anyhow!("Windows service {service_name} is not installed"))?;
            let empty: [&OsStr; 0] = [];
            match service.start(&empty) {
                Ok(()) => {}
                Err(err) if windows_service_error_code(&err) == Some(1056) => {}
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("failed to start Windows service {service_name}")
                    });
                }
            }
            windows_wait_service_running(&service_name)
        }
    }
}

#[cfg(windows)]
pub(super) fn platform_stop(plan: &ServicePlan) -> Result<()> {
    let service_name = platform_service_name(plan.scope);
    match plan.scope {
        ServiceScope::User => run_command("schtasks", &["/End", "/TN", &service_name]),
        ServiceScope::System => {
            let Some(service) = windows_open_system_service(
                &service_name,
                ServiceAccess::STOP | ServiceAccess::QUERY_STATUS,
            )?
            else {
                return Ok(());
            };
            match service.stop() {
                Ok(_) => windows_wait_service_stopped(&service_name),
                Err(err) if windows_service_error_code(&err) == Some(1062) => Ok(()),
                Err(err) => Err(err)
                    .with_context(|| format!("failed to stop Windows service {service_name}")),
            }
        }
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
fn windows_service_exists(service_name: &str) -> bool {
    windows_open_system_service(service_name, ServiceAccess::QUERY_STATUS)
        .map(|service| service.is_some())
        .unwrap_or(false)
}

#[cfg(windows)]
fn windows_service_running(service_name: &str) -> bool {
    windows_open_system_service(service_name, ServiceAccess::QUERY_STATUS)
        .ok()
        .flatten()
        .is_some_and(|service| windows_service_status_is(&service, ServiceState::Running))
}

#[cfg(windows)]
fn windows_stop_service_for_replace(service_name: &str) -> Result<()> {
    if !windows_service_exists(service_name) {
        return Ok(());
    }
    if !windows_service_stopped(service_name) {
        if let Some(service) = windows_open_system_service(
            service_name,
            ServiceAccess::STOP | ServiceAccess::QUERY_STATUS,
        )? {
            match service.stop() {
                Ok(_) => {}
                Err(err) if windows_service_error_code(&err) == Some(1062) => {}
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("failed to stop Windows service {service_name}"));
                }
            }
        }
    }
    windows_wait_service_stopped(service_name)
}

#[cfg(windows)]
fn windows_wait_service_stopped(service_name: &str) -> Result<()> {
    for _ in 0..60 {
        if windows_service_stopped(service_name) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }
    bail!("Windows service {service_name} did not stop before binary replacement")
}

#[cfg(windows)]
fn windows_service_stopped(service_name: &str) -> bool {
    windows_open_system_service(service_name, ServiceAccess::QUERY_STATUS)
        .map(|service| {
            service.is_none()
                || service.is_some_and(|service| {
                    windows_service_status_is(&service, ServiceState::Stopped)
                })
        })
        .unwrap_or(false)
}

#[cfg(windows)]
fn windows_wait_service_running(service_name: &str) -> Result<()> {
    for _ in 0..80 {
        if windows_service_running(service_name) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }
    let status = capture_command_output("sc.exe", &["query", service_name]);
    bail!(
        "Windows service {service_name} did not reach RUNNING state: {}{}",
        status["stdout"].as_str().unwrap_or_default(),
        status["stderr"].as_str().unwrap_or_default()
    )
}

#[cfg(windows)]
fn windows_task_running(service_name: &str) -> bool {
    capture_command_output(
        "schtasks",
        &["/Query", "/TN", service_name, "/FO", "LIST", "/V"],
    )["stdout"]
        .as_str()
        .map(|stdout| stdout.to_ascii_lowercase().contains("running"))
        .unwrap_or(false)
}

#[cfg(windows)]
fn windows_service_manager(access: ServiceManagerAccess) -> Result<ServiceManager> {
    ServiceManager::local_computer(None::<&str>, access)
        .context("failed to connect to Windows Service Control Manager")
}

#[cfg(windows)]
fn windows_open_system_service(
    service_name: &str,
    access: ServiceAccess,
) -> Result<Option<Service>> {
    let manager = windows_service_manager(ServiceManagerAccess::CONNECT)?;
    match manager.open_service(service_name, access) {
        Ok(service) => Ok(Some(service)),
        Err(err) if windows_service_error_code(&err) == Some(1060) => Ok(None),
        Err(err) => {
            Err(err).with_context(|| format!("failed to open Windows service {service_name}"))
        }
    }
}

#[cfg(windows)]
fn windows_system_service_info(plan: &ServicePlan, service_name: &str) -> ServiceInfo {
    ServiceInfo {
        name: OsString::from(service_name),
        display_name: OsString::from("ssh_proxy daemon"),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: plan.exe.clone(),
        launch_arguments: windows_service_launch_arguments(plan),
        dependencies: Vec::new(),
        account_name: None,
        account_password: None,
    }
}

#[cfg(windows)]
fn windows_service_launch_arguments(plan: &ServicePlan) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("daemon"),
        OsString::from("serve"),
        OsString::from("--control"),
        OsString::from(&plan.endpoint),
    ];
    if let Some(transport) = plan.transport {
        args.push(OsString::from("--transport"));
        args.push(OsString::from(transport.to_string()));
    }
    if let Some(token) = &plan.token {
        args.push(OsString::from("--token"));
        args.push(OsString::from(token));
    }
    if let Some(transport) = plan.tls_transport {
        args.push(OsString::from("--tls-transport"));
        args.push(OsString::from(transport.to_string()));
    }
    if let Some(transport) = plan.quic_transport {
        args.push(OsString::from("--quic-transport"));
        args.push(OsString::from(transport.to_string()));
    }
    if let Some(path) = &plan.tls_cert {
        args.push(OsString::from("--tls-cert"));
        args.push(path.as_os_str().to_os_string());
    }
    if let Some(path) = &plan.tls_key {
        args.push(OsString::from("--tls-key"));
        args.push(path.as_os_str().to_os_string());
    }
    if let Some(path) = &plan.tls_client_ca {
        args.push(OsString::from("--tls-client-ca"));
        args.push(path.as_os_str().to_os_string());
    }
    for endpoint in &plan.report_to {
        args.push(OsString::from("--report-to"));
        args.push(OsString::from(endpoint));
    }
    args
}

#[cfg(windows)]
fn windows_service_status_is(service: &Service, expected: ServiceState) -> bool {
    service
        .query_status()
        .map(|status| status.current_state == expected)
        .unwrap_or(false)
}

#[cfg(windows)]
fn windows_service_error_code(error: &WindowsServiceError) -> Option<i32> {
    match error {
        WindowsServiceError::Winapi(err) => err.raw_os_error(),
        _ => None,
    }
}

#[cfg(windows)]
fn is_elevated_for_platform() -> bool {
    crate::service::plan::is_admin()
}

#[cfg(windows)]
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn wide_os(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn join_windows_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.chars().any(|ch| ch.is_whitespace() || ch == '"') {
                format!("\"{}\"", arg.replace('"', "\\\""))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(windows)]
fn powershell_array(args: &[String]) -> String {
    format!(
        "@({})",
        args.iter()
            .map(|arg| powershell_quote(arg))
            .collect::<Vec<_>>()
            .join(",")
    )
}

#[cfg(windows)]
fn windows_sc_create(plan: &ServicePlan, service_name: &str) -> String {
    format!(
        "sc.exe create {service_name} start= auto DisplayName= \"ssh_proxy daemon\" binPath= {}",
        command_quote(&plan.daemon_command())
    )
}
