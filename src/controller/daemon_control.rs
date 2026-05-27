use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{Mutex, Notify},
    task::JoinHandle,
};
use tracing::{error, info, warn};

use crate::{config, control_socket, sidecar};

use super::run as run_proxy;

pub(super) async fn run(
    endpoint: control_socket::ControlEndpoint,
    config: config::AppConfig,
) -> Result<()> {
    let manager = Arc::new(DaemonManager::new(config));
    run_control_server(endpoint, manager).await
}

struct DaemonManager {
    config: config::AppConfig,
    started: Instant,
    instances: Mutex<HashMap<String, JoinHandle<()>>>,
    shutdown: AtomicBool,
    shutdown_notify: Notify,
}

impl DaemonManager {
    fn new(config: config::AppConfig) -> Self {
        Self {
            config,
            started: Instant::now(),
            instances: Mutex::new(HashMap::new()),
            shutdown: AtomicBool::new(false),
            shutdown_notify: Notify::new(),
        }
    }

    async fn connect_profile(&self, profile: &str) -> Result<String> {
        let mut instances = self.instances.lock().await;
        instances.retain(|_, handle| !handle.is_finished());
        if instances.contains_key(profile) {
            return Ok(format!("profile {profile:?} is already running"));
        }
        let mut args = self.config.proxy_from_profile(profile)?;
        args.control_listen = None;
        let name = profile.to_string();
        let task_name = name.clone();
        let handle = tokio::spawn(async move {
            if let Err(err) = run_proxy(args).await {
                error!(profile = %task_name, error = %err, "daemon proxy instance stopped");
            }
        });
        instances.insert(name, handle);
        Ok(format!("profile {profile:?} started"))
    }

    async fn disconnect_profile(&self, profile: &str) -> Result<String> {
        let mut instances = self.instances.lock().await;
        if let Some(handle) = instances.remove(profile) {
            handle.abort();
            Ok(format!("profile {profile:?} stopped"))
        } else {
            bail!("profile {profile:?} is not running");
        }
    }

    async fn status_json(&self) -> Result<String> {
        #[derive(Serialize)]
        struct DaemonStatus {
            ok: bool,
            version: &'static str,
            os: &'static str,
            arch: &'static str,
            linux_musl_sidecar: &'static str,
            uptime_secs: u64,
            config_path: String,
            profiles: Vec<String>,
            running: Vec<String>,
        }

        let profiles = self.config.profiles.keys().cloned().collect::<Vec<_>>();
        let mut instances = self.instances.lock().await;
        instances.retain(|_, handle| !handle.is_finished());
        let running = instances.keys().cloned().collect::<Vec<_>>();
        let status = DaemonStatus {
            ok: true,
            version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            linux_musl_sidecar: sidecar::build_summary(),
            uptime_secs: self.started.elapsed().as_secs(),
            config_path: config::config_path()?.display().to_string(),
            profiles,
            running,
        };
        Ok(format!("{}\n", serde_json::to_string_pretty(&status)?))
    }

    async fn shutdown(&self) -> Result<String> {
        self.shutdown.store(true, Ordering::Relaxed);
        let mut instances = self.instances.lock().await;
        for (_, handle) in instances.drain() {
            handle.abort();
        }
        self.shutdown_notify.notify_waiters();
        Ok("{\"ok\":true,\"message\":\"daemon shutdown requested\"}\n".to_string())
    }

    async fn shutdown_notified(&self) {
        if self.shutdown.load(Ordering::Relaxed) {
            return;
        }
        self.shutdown_notify.notified().await;
    }
}

#[derive(Deserialize)]
struct DaemonRequest {
    cmd: String,
    profile: Option<String>,
}

async fn run_control_server(
    endpoint: control_socket::ControlEndpoint,
    manager: Arc<DaemonManager>,
) -> Result<()> {
    let listener = control_socket::ControlListener::bind(&endpoint).await?;
    info!(%endpoint, "daemon control listener ready");
    loop {
        tokio::select! {
            accept = listener.accept() => {
                let stream = accept?;
                let manager = manager.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_control(stream, manager).await {
                        warn!(error = %err, "daemon control request failed");
                    }
                });
            }
            _ = manager.shutdown_notified() => break,
        }
    }
    Ok(())
}

async fn handle_control(
    stream: control_socket::ControlStream,
    manager: Arc<DaemonManager>,
) -> Result<()> {
    let mut reader = BufReader::new(stream);
    let mut command = String::new();
    reader.read_line(&mut command).await?;
    let mut stream = reader.into_inner();
    let response = match parse_request(&command) {
        Ok(request) => match request.cmd.to_ascii_lowercase().as_str() {
            "status" | "" => manager.status_json().await?,
            "shutdown" => manager.shutdown().await?,
            "connect" => {
                let profile = request
                    .profile
                    .ok_or_else(|| anyhow!("connect requires a profile"))?;
                let message = manager.connect_profile(&profile).await?;
                format!("{}\n", serde_json::json!({"ok": true, "message": message}))
            }
            "disconnect" => {
                let profile = request
                    .profile
                    .ok_or_else(|| anyhow!("disconnect requires a profile"))?;
                let message = manager.disconnect_profile(&profile).await?;
                format!("{}\n", serde_json::json!({"ok": true, "message": message}))
            }
            other => format!(
                "{}\n",
                serde_json::json!({
                    "ok": false,
                    "error": format!("unknown daemon command {other:?}")
                })
            ),
        },
        Err(err) => format!(
            "{}\n",
            serde_json::json!({"ok": false, "error": err.to_string()})
        ),
    };
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await.ok();
    Ok(())
}

fn parse_request(command: &str) -> Result<DaemonRequest> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Ok(DaemonRequest {
            cmd: "status".to_string(),
            profile: None,
        });
    }
    if trimmed.starts_with('{') {
        return serde_json::from_str(trimmed).context("failed to parse daemon JSON request");
    }
    Ok(DaemonRequest {
        cmd: trimmed.to_string(),
        profile: None,
    })
}
