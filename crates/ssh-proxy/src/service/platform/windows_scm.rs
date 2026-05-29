use std::{
    ffi::{OsStr, OsString},
    os::windows::ffi::OsStrExt,
    path::Path,
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use ssh_proxy_core::external::ExternalActionClass;
use ssh_proxy_platform::windows_service::{
    Error as WindowsServiceError,
    service::{
        Service, ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState,
        ServiceType,
    },
    service_manager::{ServiceManager, ServiceManagerAccess},
};
use ssh_proxy_platform::windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError},
    System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
    UI::{
        Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::SW_HIDE,
    },
};
use ssh_proxy_platform::{PlatformCommandPlan, capture_command};

use crate::{
    install_report,
    service::{
        inventory::{ServiceProbeState, ServiceProbeSummary},
        plan::{ServicePlan, ServiceScope, command_quote, ensure_admin},
    },
};

use super::{
    command::{capture_command_output, run_command_output},
    probe::{contains_permission_denied, service_probe_summary},
};

pub(super) fn print(plan: &ServicePlan, service_name: &str) {
    println!("Windows system service:");
    println!("  {}", sc_create(plan, service_name));
    println!("  sc.exe start {service_name}");
    println!("  sc.exe query {service_name}");
}

pub(super) fn probe_summary(scope: ServiceScope, service_name: String) -> ServiceProbeSummary {
    let capture = native_status_summary(&service_name);
    let stderr = capture["stderr"].as_str().unwrap_or_default();
    let running = capture["running"].as_bool().unwrap_or(false);
    let exists = capture["exists"].as_bool().unwrap_or(false);
    let permission_denied =
        capture["permission_denied"].as_bool().unwrap_or(false) || contains_permission_denied(stderr);
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
            "execution_backend": capture["execution_backend"].clone(),
            "native_api_available": capture["native_api_available"].clone(),
            "fallback_used": capture["fallback_used"].clone(),
            "capture": capture,
            "running": running,
        }),
    )
}

pub(super) fn install_requires_elevation(plan: &ServicePlan) -> bool {
    plan.elevate && !is_elevated_for_platform()
}

pub(super) fn prepare_install(service_name: &str) -> Result<()> {
    ensure_admin("preparing a Windows system service install requires administrator privileges")?;
    stop_service_for_replace(service_name)
}

pub(super) fn install(plan: &ServicePlan, service_name: &str) -> Result<()> {
    if plan.elevate && !is_elevated_for_platform() {
        return install_elevated(plan);
    }
    ensure_admin("installing a Windows system service requires administrator privileges")?;
    let manager =
        service_manager(ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)?;
    let service_info = system_service_info(plan, service_name);
    let access = ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::QUERY_STATUS
        | ServiceAccess::START
        | ServiceAccess::STOP;
    let service = match manager.open_service(service_name, access) {
        Ok(service) => {
            service
                .change_config(&service_info)
                .with_context(|| format!("failed to configure Windows service {service_name}"))?;
            service
        }
        Err(err) if service_error_code(&err) == Some(1060) => manager
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
    if service_status_is(&service, ServiceState::Running) {
        Ok(())
    } else {
        start(plan, service_name)
    }
}

pub(super) fn uninstall(service_name: &str) -> Result<()> {
    ensure_admin("removing a Windows system service requires administrator privileges")?;
    if let Some(service) = open_system_service(
        service_name,
        ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS | ServiceAccess::STOP,
    )? {
        if !service_status_is(&service, ServiceState::Stopped) {
            let _ = service.stop();
            wait_service_stopped(service_name)?;
        }
        service
            .delete()
            .with_context(|| format!("failed to delete Windows service {service_name}"))?;
    }
    Ok(())
}

pub(super) fn install_elevated(plan: &ServicePlan) -> Result<()> {
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

pub(super) fn start(plan: &ServicePlan, service_name: &str) -> Result<()> {
    if plan.elevate && !is_elevated_for_platform() {
        return install_elevated(plan);
    }
    if service_running(service_name) {
        return Ok(());
    }
    let service = open_system_service(
        service_name,
        ServiceAccess::START | ServiceAccess::QUERY_STATUS,
    )?
    .ok_or_else(|| anyhow::anyhow!("Windows service {service_name} is not installed"))?;
    let empty: [&OsStr; 0] = [];
    match service.start(&empty) {
        Ok(()) => {}
        Err(err) if service_error_code(&err) == Some(1056) => {}
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed to start Windows service {service_name}"));
        }
    }
    wait_service_running(service_name)
}

pub(super) fn stop(service_name: &str) -> Result<()> {
    let Some(service) = open_system_service(
        service_name,
        ServiceAccess::STOP | ServiceAccess::QUERY_STATUS,
    )?
    else {
        return Ok(());
    };
    match service.stop() {
        Ok(_) => wait_service_stopped(service_name),
        Err(err) if service_error_code(&err) == Some(1062) => Ok(()),
        Err(err) => {
            Err(err).with_context(|| format!("failed to stop Windows service {service_name}"))
        }
    }
}

#[allow(dead_code)]
pub(super) fn status(service_name: &str) -> Result<()> {
    run_command_output("sc.exe", &["query", service_name])
}

pub(super) fn status_summary(service_name: &str) -> Value {
    native_status_summary(service_name)
}

fn service_exists(service_name: &str) -> bool {
    open_system_service(service_name, ServiceAccess::QUERY_STATUS)
        .map(|service| service.is_some())
        .unwrap_or(false)
}

fn service_running(service_name: &str) -> bool {
    open_system_service(service_name, ServiceAccess::QUERY_STATUS)
        .ok()
        .flatten()
        .is_some_and(|service| service_status_is(&service, ServiceState::Running))
}

fn native_status_summary(service_name: &str) -> Value {
    match open_system_service(service_name, ServiceAccess::QUERY_STATUS) {
        Ok(Some(service)) => match service.query_status() {
            Ok(status) => {
                let running = status.current_state == ServiceState::Running;
                json!({
                    "ok": true,
                    "program": Value::Null,
                    "args": [],
                    "class": ExternalActionClass::RequiredProvider.as_str(),
                    "execution_backend": "native_api",
                    "native_api_available": true,
                    "fallback_used": false,
                    "reason": "query Windows SCM status through windows-service API",
                    "service_name": service_name,
                    "exists": true,
                    "running": running,
                    "state": format!("{:?}", status.current_state),
                    "status_code": Value::Null,
                    "stdout": "",
                    "stderr": "",
                    "permission_denied": false,
                })
            }
            Err(err) => native_status_fallback(service_name, Some(err.to_string())),
        },
        Ok(None) => json!({
            "ok": true,
            "program": Value::Null,
            "args": [],
            "class": ExternalActionClass::RequiredProvider.as_str(),
            "execution_backend": "native_api",
            "native_api_available": true,
            "fallback_used": false,
            "reason": "query Windows SCM status through windows-service API",
            "service_name": service_name,
            "exists": false,
            "running": false,
            "state": "NotInstalled",
            "status_code": Value::Null,
            "stdout": "",
            "stderr": "",
            "permission_denied": false,
        }),
        Err(err) => native_status_fallback(service_name, Some(err.to_string())),
    }
}

fn native_status_fallback(service_name: &str, native_error: Option<String>) -> Value {
    let mut fallback = capture_command_output("sc.exe", &["query", service_name]);
    let stdout = fallback["stdout"].as_str().unwrap_or_default().to_string();
    let stderr = fallback["stderr"].as_str().unwrap_or_default().to_string();
    let running = stdout.to_ascii_uppercase().contains("RUNNING");
    let exists = fallback["ok"].as_bool().unwrap_or(false);
    if let Some(object) = fallback.as_object_mut() {
        object.insert("execution_backend".to_string(), json!("provider_command"));
        object.insert("native_api_available".to_string(), json!(false));
        object.insert("fallback_used".to_string(), json!(true));
        object.insert("native_error".to_string(), json!(native_error));
        object.insert("service_name".to_string(), json!(service_name));
        object.insert("exists".to_string(), json!(exists));
        object.insert("running".to_string(), json!(running));
        object.insert(
            "permission_denied".to_string(),
            json!(contains_permission_denied(&stderr)),
        );
    }
    fallback
}

fn stop_service_for_replace(service_name: &str) -> Result<()> {
    if !service_exists(service_name) {
        return Ok(());
    }
    if !service_stopped(service_name)
        && let Some(service) = open_system_service(
            service_name,
            ServiceAccess::STOP | ServiceAccess::QUERY_STATUS,
        )?
    {
        match service.stop() {
            Ok(_) => {}
            Err(err) if service_error_code(&err) == Some(1062) => {}
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to stop Windows service {service_name}"));
            }
        }
    }
    wait_service_stopped(service_name)
}

fn wait_service_stopped(service_name: &str) -> Result<()> {
    for _ in 0..60 {
        if service_stopped(service_name) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }
    bail!("Windows service {service_name} did not stop before binary replacement")
}

fn service_stopped(service_name: &str) -> bool {
    open_system_service(service_name, ServiceAccess::QUERY_STATUS)
        .map(|service| {
            service.is_none()
                || service.is_some_and(|service| service_status_is(&service, ServiceState::Stopped))
        })
        .unwrap_or(false)
}

fn wait_service_running(service_name: &str) -> Result<()> {
    for _ in 0..80 {
        if service_running(service_name) {
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

fn service_manager(access: ServiceManagerAccess) -> Result<ServiceManager> {
    ServiceManager::local_computer(None::<&str>, access)
        .context("failed to connect to Windows Service Control Manager")
}

fn open_system_service(service_name: &str, access: ServiceAccess) -> Result<Option<Service>> {
    let manager = service_manager(ServiceManagerAccess::CONNECT)?;
    match manager.open_service(service_name, access) {
        Ok(service) => Ok(Some(service)),
        Err(err) if service_error_code(&err) == Some(1060) => Ok(None),
        Err(err) => {
            Err(err).with_context(|| format!("failed to open Windows service {service_name}"))
        }
    }
}

fn system_service_info(plan: &ServicePlan, service_name: &str) -> ServiceInfo {
    ServiceInfo {
        name: OsString::from(service_name),
        display_name: OsString::from("ssh_proxy daemon"),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: plan.exe.clone(),
        launch_arguments: service_launch_arguments(plan),
        dependencies: Vec::new(),
        account_name: None,
        account_password: None,
    }
}

fn service_launch_arguments(plan: &ServicePlan) -> Vec<OsString> {
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

fn service_status_is(service: &Service, expected: ServiceState) -> bool {
    service
        .query_status()
        .map(|status| status.current_state == expected)
        .unwrap_or(false)
}

fn service_error_code(error: &WindowsServiceError) -> Option<i32> {
    match error {
        WindowsServiceError::Winapi(err) => err.raw_os_error(),
        _ => None,
    }
}

fn is_elevated_for_platform() -> bool {
    crate::service::plan::is_admin()
}

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
    let plan = PlatformCommandPlan::new(
        "powershell.exe",
        [
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &command,
        ],
        ExternalActionClass::RequiredProvider,
        "run elevated PowerShell installer fallback for Windows service install",
    )
    .with_repair_action("approve the elevation prompt or rerun from an elevated shell");
    let outcome =
        capture_command(plan).context("failed to run elevated PowerShell installer fallback")?;
    Ok(outcome.status_code.unwrap_or(1) as u32)
}

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

fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

fn wide_os(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

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

fn powershell_array(args: &[String]) -> String {
    format!(
        "@({})",
        args.iter()
            .map(|arg| powershell_quote(arg))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn sc_create(plan: &ServicePlan, service_name: &str) -> String {
    format!(
        "sc.exe create {service_name} start= auto DisplayName= \"ssh_proxy daemon\" binPath= {}",
        command_quote(&plan.daemon_command())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powershell_quote_doubles_single_quotes() {
        assert_eq!(powershell_quote("a'b"), "'a''b'");
    }

    #[test]
    fn sc_create_preserves_service_contract() {
        let command = sc_create_for_test("ssh_proxy daemon serve", "ssh_proxy");

        assert!(command.contains("sc.exe create ssh_proxy"));
        assert!(command.contains("start= auto"));
        assert!(command.contains("binPath="));
    }

    fn sc_create_for_test(daemon_command: &str, service_name: &str) -> String {
        format!(
            "sc.exe create {service_name} start= auto DisplayName= \"ssh_proxy daemon\" binPath= {}",
            command_quote(daemon_command)
        )
    }
}
