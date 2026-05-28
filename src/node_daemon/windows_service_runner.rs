use std::{
    ffi::OsString,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use anyhow::{Context, Result};
use tokio::sync::oneshot;
use windows_service::{
    Error as WindowsServiceError, define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};

use crate::{cli, config};

const SERVICE_NAME: &str = "ssh_proxy";

static SERVICE_CONTEXT: OnceLock<Mutex<Option<ServiceContext>>> = OnceLock::new();

struct ServiceContext {
    args: cli::NodeDaemonArgs,
    config: config::AppConfig,
}

define_windows_service!(ffi_service_main, service_main);

pub(crate) fn run_if_started_by_scm(
    args: cli::NodeDaemonArgs,
    config: config::AppConfig,
) -> Result<bool> {
    let context = SERVICE_CONTEXT.get_or_init(|| Mutex::new(None));
    {
        let mut guard = context.lock().expect("service context mutex poisoned");
        *guard = Some(ServiceContext { args, config });
    }

    match service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        Ok(()) => Ok(true),
        Err(err) if windows_service_error_code(&err) == Some(1063) => {
            let mut guard = context.lock().expect("service context mutex poisoned");
            let _ = guard.take();
            Ok(false)
        }
        Err(err) => Err(err).context("failed to start Windows service dispatcher"),
    }
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(err) = run_service() {
        eprintln!("ssh_proxy Windows service failed: {err:?}");
    }
}

fn run_service() -> Result<()> {
    let context = SERVICE_CONTEXT
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("service context mutex poisoned")
        .take()
        .context("missing ssh_proxy Windows service context")?;

    let stop_sender = Arc::new(Mutex::new(None));
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    *stop_sender.lock().expect("stop sender mutex poisoned") = Some(shutdown_tx);
    let handler_sender = stop_sender.clone();

    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            ServiceControl::Stop | ServiceControl::Shutdown => {
                if let Some(sender) = handler_sender
                    .lock()
                    .expect("stop sender mutex poisoned")
                    .take()
                {
                    let _ = sender.send(());
                }
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)
        .context("failed to register Windows service control handler")?;

    status_handle
        .set_service_status(service_status(
            ServiceState::StartPending,
            ServiceControlAccept::empty(),
            1,
            Duration::from_secs(30),
        ))
        .context("failed to report Windows service start-pending status")?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to create Windows service Tokio runtime")?;

    status_handle
        .set_service_status(service_status(
            ServiceState::Running,
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            0,
            Duration::default(),
        ))
        .context("failed to report Windows service running status")?;

    let run_result = runtime.block_on(super::run_daemon_inner(
        context.args,
        context.config,
        Some(shutdown_rx),
    ));

    status_handle
        .set_service_status(service_status(
            ServiceState::StopPending,
            ServiceControlAccept::empty(),
            1,
            Duration::from_secs(10),
        ))
        .ok();

    let exit_code = if run_result.is_ok() { 0 } else { 1 };
    status_handle
        .set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::Stopped,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(exit_code),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })
        .ok();

    run_result
}

fn service_status(
    state: ServiceState,
    controls_accepted: ServiceControlAccept,
    checkpoint: u32,
    wait_hint: Duration,
) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint,
        wait_hint,
        process_id: None,
    }
}

fn windows_service_error_code(error: &WindowsServiceError) -> Option<i32> {
    match error {
        WindowsServiceError::Winapi(err) => err.raw_os_error(),
        _ => None,
    }
}
