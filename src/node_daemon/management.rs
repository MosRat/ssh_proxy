use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, anyhow};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::paths;

use super::{NodeManager, NodeRequest, NodeResponse, jobs, response_line};

#[derive(Debug, Clone)]
struct StagedUpdate {
    source: PathBuf,
    staged_path: PathBuf,
    hash: String,
    version: String,
}

#[derive(Debug, Clone)]
struct UpdateSwitchPlan {
    service_name: String,
    current_exe: PathBuf,
    backup_path: PathBuf,
    script_path: PathBuf,
}

impl NodeManager {
    pub(super) async fn daemon_update(&self, request: NodeRequest) -> Result<String> {
        let source = request.update_source.clone();
        self.state
            .record_daemon_update_requested(source.clone())
            .await?;
        let job = jobs::JobRecord::new("self-update:pending", "self_update")
            .transition(jobs::JobState::Running, jobs::JobPhase::StageUpdate, 10)
            .with_next_action("stage_new_binary");
        let job = self
            .jobs
            .upsert(job, "daemon self-update requested")
            .await?;
        let staged = match source.as_deref() {
            Some(source) => match stage_update_binary(Path::new(source)) {
                Ok(staged) => staged,
                Err(err) => {
                    let error = err.to_string();
                    let update_state = self
                        .state
                        .record_daemon_update_state(
                            "failed",
                            Some(source.to_string()),
                            None,
                            None,
                            None,
                            None,
                            None,
                            Some(error.clone()),
                        )
                        .await?;
                    let job = jobs::JobRecord::new(job.id.clone(), "self_update")
                        .transition(jobs::JobState::Failed, jobs::JobPhase::Failed, 100)
                        .failed(error, Some("update_stage_failed".to_string()))
                        .with_next_action("pass a readable ssh_proxy binary with --source");
                    let job = self
                        .jobs
                        .upsert(job, "daemon self-update staging failed")
                        .await?;
                    return response_line(json!({
                        "ok": false,
                        "kind": "daemon_update",
                        "daemon_api": "v0.3",
                        "job": job.to_value(),
                        "update_state": update_state,
                        "requires_daemon": true,
                    }));
                }
            },
            None => {
                let error = "daemon update requires --source in v0.3 staged mode".to_string();
                let update_state = self
                    .state
                    .record_daemon_update_state(
                        "blocked",
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        Some(error.clone()),
                    )
                    .await?;
                let job = jobs::JobRecord::new(job.id.clone(), "self_update")
                    .failed(error, Some("missing_update_source".to_string()))
                    .with_next_action("ssh_proxy daemon update --source <path-to-new-binary>");
                let job = self
                    .jobs
                    .upsert(job, "daemon self-update missing source")
                    .await?;
                return response_line(json!({
                    "ok": false,
                    "kind": "daemon_update",
                    "daemon_api": "v0.3",
                    "job": job.to_value(),
                    "update_state": update_state,
                    "requires_daemon": true,
                }));
            }
        };
        let job = self
            .jobs
            .upsert(
                jobs::JobRecord::new(job.id.clone(), "self_update")
                    .transition(jobs::JobState::Running, jobs::JobPhase::VerifyUpdate, 55)
                    .with_next_action("verify_staged_binary"),
                "daemon self-update binary staged",
            )
            .await?;
        self.state
            .record_daemon_update_state(
                "staged",
                Some(staged.source.display().to_string()),
                Some(staged.staged_path.display().to_string()),
                Some(staged.hash.clone()),
                Some(staged.version.clone()),
                None,
                None,
                None,
            )
            .await?;
        let switch_plan = match prepare_update_switch(&staged) {
            Ok(plan) => plan,
            Err(err) => {
                let error = err.to_string();
                let update_state = self
                    .state
                    .record_daemon_update_state(
                        "failed",
                        Some(staged.source.display().to_string()),
                        Some(staged.staged_path.display().to_string()),
                        Some(staged.hash.clone()),
                        Some(staged.version.clone()),
                        None,
                        None,
                        Some(error.clone()),
                    )
                    .await?;
                let job = self
                    .jobs
                    .upsert(
                        jobs::JobRecord::new(job.id.clone(), "self_update")
                            .transition(jobs::JobState::Failed, jobs::JobPhase::Failed, 100)
                            .failed(error, Some("update_switch_prepare_failed".to_string()))
                            .with_next_action(
                                "run ssh_proxy daemon install --scope system --elevate",
                            ),
                        "daemon self-update switch preparation failed",
                    )
                    .await?;
                return response_line(json!({
                    "ok": false,
                    "kind": "daemon_update",
                    "daemon_api": "v0.3",
                    "job": job.to_value(),
                    "update_state": update_state,
                    "requires_daemon": true,
                }));
            }
        };
        let job = self
            .jobs
            .upsert(
                jobs::JobRecord::new(job.id.clone(), "self_update")
                    .transition(jobs::JobState::Running, jobs::JobPhase::SwitchBinary, 70)
                    .with_next_action("launch_supervised_update_switch"),
                "daemon self-update switch script prepared",
            )
            .await?;
        if let Err(err) = launch_update_switch(&switch_plan) {
            let error = err.to_string();
            let update_state = self
                .state
                .record_daemon_update_state(
                    "failed",
                    Some(staged.source.display().to_string()),
                    Some(staged.staged_path.display().to_string()),
                    Some(staged.hash.clone()),
                    Some(staged.version.clone()),
                    Some(switch_plan.script_path.display().to_string()),
                    Some(switch_plan.backup_path.display().to_string()),
                    Some(error.clone()),
                )
                .await?;
            let job = self
                .jobs
                .upsert(
                    jobs::JobRecord::new(job.id.clone(), "self_update")
                        .transition(jobs::JobState::Failed, jobs::JobPhase::Failed, 100)
                        .failed(error, Some("update_switch_launch_failed".to_string()))
                        .with_next_action("run ssh_proxy daemon install --scope system --elevate"),
                    "daemon self-update switch launch failed",
                )
                .await?;
            return response_line(json!({
                "ok": false,
                "kind": "daemon_update",
                "daemon_api": "v0.3",
                "job": job.to_value(),
                "update_state": update_state,
                "requires_daemon": true,
            }));
        }
        let update_state = self
            .state
            .record_daemon_update_state(
                "restart_daemon",
                Some(staged.source.display().to_string()),
                Some(staged.staged_path.display().to_string()),
                Some(staged.hash.clone()),
                Some(staged.version.clone()),
                Some(switch_plan.script_path.display().to_string()),
                Some(switch_plan.backup_path.display().to_string()),
                None,
            )
            .await?;
        let job = self
            .jobs
            .upsert(
                jobs::JobRecord::new(job.id.clone(), "self_update")
                    .transition(
                        jobs::JobState::WaitingRetry,
                        jobs::JobPhase::RestartDaemon,
                        80,
                    )
                    .with_next_action(
                        "wait for daemon service restart and control endpoint health",
                    ),
                "daemon self-update switch launched; waiting for restart",
            )
            .await?;
        response_line(json!({
            "ok": true,
            "kind": "daemon_update",
            "daemon_api": "v0.3",
            "job": job.to_value(),
            "update_state": update_state,
            "switch": {
                "service_name": switch_plan.service_name,
                "current_exe": switch_plan.current_exe,
                "backup_path": switch_plan.backup_path,
                "script_path": switch_plan.script_path,
            },
            "requires_daemon": true,
        }))
    }

    pub(super) async fn nodes_json(&self) -> Result<String> {
        let instances = self.instances.lock().await;
        let profiles = instances
            .iter()
            .map(|(name, handle)| {
                json!({
                    "id": format!("profile:{name}"),
                    "name": name,
                    "scope": "user",
                    "state": if handle.is_finished() { "stopped" } else { "running" },
                    "managed_by": "current-daemon",
                    "kind": "profile-instance",
                })
            })
            .collect::<Vec<_>>();
        response_line(json!({
            "ok": true,
            "kind": "nodes",
            "broker_api": "v0.2",
            "nodes": [{
                "id": "current",
                "name": self.name,
                "scope": "current",
                "state": "running",
                "managed_by": "current-daemon",
                "control_endpoint": self.control_endpoint.to_string(),
                "transport": self.transport.map(|addr| addr.to_string()),
                "tls_transport": self.tls_transport.map(|addr| addr.to_string()),
                "quic_transport": self.quic_transport.map(|addr| addr.to_string()),
                "pid": std::process::id(),
                "capabilities": [
                    "route_intent",
                    "route_readiness",
                    "peer_ensure",
                    "peer_update",
                    "jobs",
                    "profile_instances"
                ],
            }],
            "profile_instances": profiles,
        }))
    }

    pub(super) async fn jobs_json(&self) -> Result<String> {
        let job_values = self.jobs.jobs_value().await;
        response_line(json!({
            "ok": true,
            "kind": "jobs",
            "daemon_api": "v0.3",
            "jobs": job_values,
            "message": "proxy session jobs are the stable v0.3 progress surface",
        }))
    }

    pub(super) async fn job_events_json(&self, request: NodeRequest) -> Result<String> {
        let events = self
            .jobs
            .events(request.id.as_deref())
            .await
            .into_iter()
            .map(|event| serde_json::to_value(event).unwrap_or_else(|_| json!({})))
            .collect::<Vec<_>>();
        response_line(json!({
            "ok": true,
            "kind": "job_events",
            "daemon_api": "v0.3",
            "job_id": request.id,
            "events": events,
        }))
    }

    pub(super) async fn job_status_json(&self, request: NodeRequest) -> Result<String> {
        let id = request
            .id
            .ok_or_else(|| anyhow!("job_status requires id"))?;
        let job = self.jobs.get(&id).await;
        let ok = job.is_some();
        let code = if ok {
            serde_json::Value::Null
        } else {
            json!("not_found")
        };
        response_line(json!({
            "ok": ok,
            "kind": "job_status",
            "daemon_api": "v0.3",
            "job": job.map(|job| job.to_value()),
            "code": code,
        }))
    }

    pub(super) async fn node_ensure(&self, request: NodeRequest) -> Result<String> {
        let scope = request.node_scope.as_deref().unwrap_or("user");
        let state = if scope == "session" {
            "session-ready"
        } else {
            "running"
        };
        response_line(json!({
            "ok": true,
            "kind": "node_ensure",
            "broker_api": "v0.2",
            "changed": false,
            "requested_scope": scope,
            "node": {
                "id": "current",
                "name": self.name,
                "scope": "current",
                "state": state,
                "control_endpoint": self.control_endpoint.to_string(),
            },
            "next_action": "reuse_current_daemon",
        }))
    }

    pub(super) async fn node_start(&self, request: NodeRequest) -> Result<String> {
        let id = request
            .id
            .ok_or_else(|| anyhow!("node_start requires id"))?;
        if is_current_node_id(&id) {
            return response_line(json!({
                "ok": true,
                "kind": "node_start",
                "changed": false,
                "id": id,
                "state": "running",
                "message": "current node daemon is already running",
            }));
        }
        response_line(json!({
            "ok": false,
            "kind": "node_start",
            "code": "unknown_node",
            "id": id,
            "state": "unavailable",
            "next_action": "node_ensure",
            "message": "this broker only manages the current daemon in v0.2 preview",
        }))
    }

    pub(super) async fn node_stop(&self, request: NodeRequest) -> Result<String> {
        let id = request.id.ok_or_else(|| anyhow!("node_stop requires id"))?;
        if is_current_node_id(&id) {
            return self.shutdown().await;
        }
        response_line(json!({
            "ok": false,
            "kind": "node_stop",
            "code": "unknown_node",
            "id": id,
            "next_action": "nodes",
        }))
    }

    pub(super) async fn node_restart(&self, request: NodeRequest) -> Result<String> {
        let id = request
            .id
            .ok_or_else(|| anyhow!("node_restart requires id"))?;
        if is_current_node_id(&id) {
            return response_line(json!({
                "ok": false,
                "kind": "node_restart",
                "code": "requires_supervisor",
                "id": id,
                "next_action": "daemon_restart_or_reinstall",
                "message": "current daemon cannot restart itself without an external broker supervisor",
            }));
        }
        response_line(json!({
            "ok": false,
            "kind": "node_restart",
            "code": "unknown_node",
            "id": id,
            "next_action": "nodes",
        }))
    }

    pub(super) async fn ensure_peer(&self, request: NodeRequest) -> Result<String> {
        let Some(args) = request.bootstrap else {
            return NodeResponse::error("bad_request", "peer_ensure requires bootstrap args")
                .to_line();
        };
        let alias = args.alias.clone().unwrap_or_else(|| args.target.clone());
        if self.peer_is_recorded(&alias).await {
            return response_line(json!({
                "ok": true,
                "kind": "peer_ensure",
                "broker_api": "v0.2",
                "alias": alias,
                "changed": false,
                "state": "ready",
                "next_action": "reuse_recorded_peer",
            }));
        }
        let response = self.bootstrap_peer_from_args(args).await?;
        let mut value: serde_json::Value = serde_json::from_str(response.trim())?;
        if let Some(object) = value.as_object_mut() {
            object.insert("kind".to_string(), json!("peer_ensure"));
            object.insert("broker_api".to_string(), json!("v0.2"));
            object.insert("state".to_string(), json!("bootstrapped"));
            object.insert("requires_external_ssh".to_string(), json!(false));
        }
        response_line(value)
    }

    pub(super) async fn update_peer(&self, request: NodeRequest) -> Result<String> {
        let Some(args) = request.bootstrap else {
            return NodeResponse::error("bad_request", "peer_update requires bootstrap args")
                .to_line();
        };
        let alias = args.alias.clone().unwrap_or_else(|| args.target.clone());
        let response = self.refresh_peer_from_args(args).await?;
        let mut value: serde_json::Value = serde_json::from_str(response.trim())?;
        if let Some(object) = value.as_object_mut() {
            object.insert("kind".to_string(), json!("peer_update"));
            object.insert("broker_api".to_string(), json!("v0.2"));
            object.insert("alias".to_string(), json!(alias));
            object.insert("state".to_string(), json!("refreshed"));
            object.insert("requires_external_ssh".to_string(), json!(false));
        }
        response_line(value)
    }
}

fn is_current_node_id(id: &str) -> bool {
    matches!(id, "current" | "local" | "self")
}

fn stage_update_binary(source: &Path) -> Result<StagedUpdate> {
    let metadata = std::fs::metadata(source)
        .with_context(|| format!("failed to read update source {}", source.display()))?;
    if !metadata.is_file() {
        anyhow::bail!("update source {} is not a file", source.display());
    }
    let hash = file_sha256_hex(source)?;
    let version = binary_version(source)?;
    let app_home = paths::app_home()?;
    let updates_dir = app_home.join("updates");
    std::fs::create_dir_all(&updates_dir).with_context(|| {
        format!(
            "failed to create update staging dir {}",
            updates_dir.display()
        )
    })?;
    let short_hash = hash.get(..12).unwrap_or(&hash);
    let staged_path = updates_dir.join(format!(
        "ssh_proxy-staged-{short_hash}{}",
        std::env::consts::EXE_SUFFIX
    ));
    std::fs::copy(source, &staged_path).with_context(|| {
        format!(
            "failed to stage update binary {} to {}",
            source.display(),
            staged_path.display()
        )
    })?;
    Ok(StagedUpdate {
        source: source.to_path_buf(),
        staged_path,
        hash,
        version,
    })
}

fn prepare_update_switch(staged: &StagedUpdate) -> Result<UpdateSwitchPlan> {
    let current_exe = std::env::current_exe().context("failed to locate running daemon binary")?;
    if same_path(&current_exe, &staged.staged_path) {
        anyhow::bail!("staged update path is the currently running binary");
    }
    let updates_dir = staged
        .staged_path
        .parent()
        .ok_or_else(|| anyhow!("staged update path has no parent directory"))?;
    let short_hash = staged.hash.get(..12).unwrap_or(&staged.hash);
    let backup_path = updates_dir.join(format!(
        "ssh_proxy-backup-{short_hash}{}",
        std::env::consts::EXE_SUFFIX
    ));
    let script_path = updates_dir.join(format!(
        "ssh_proxy-self-update-{short_hash}{}",
        if cfg!(windows) { ".ps1" } else { ".sh" }
    ));
    let plan = UpdateSwitchPlan {
        service_name: "ssh_proxy".to_string(),
        current_exe,
        backup_path,
        script_path,
    };
    write_update_switch_script(&plan, &staged.staged_path)?;
    Ok(plan)
}

fn write_update_switch_script(plan: &UpdateSwitchPlan, staged_path: &Path) -> Result<()> {
    let script = if cfg!(windows) {
        windows_update_script(plan, staged_path)
    } else {
        unix_update_script(plan, staged_path)
    };
    fs::write(&plan.script_path, script).with_context(|| {
        format!(
            "failed to write daemon self-update script {}",
            plan.script_path.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&plan.script_path)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&plan.script_path, permissions)?;
    }
    Ok(())
}

fn launch_update_switch(plan: &UpdateSwitchPlan) -> Result<()> {
    let mut command = if cfg!(windows) {
        let mut command = Command::new("powershell");
        command.args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &plan.script_path.display().to_string(),
        ]);
        command
    } else {
        let mut command = Command::new("sh");
        command.arg(&plan.script_path);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| {
            format!(
                "failed to launch daemon self-update script {}",
                plan.script_path.display()
            )
        })?;
    Ok(())
}

fn windows_update_script(plan: &UpdateSwitchPlan, staged_path: &Path) -> String {
    format!(
        r#"$ErrorActionPreference = 'Stop'
Start-Sleep -Milliseconds 750
$serviceName = {service}
$current = {current}
$staged = {staged}
$backup = {backup}
try {{
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $backup) | Out-Null
  if (Test-Path -LiteralPath $current) {{
    Copy-Item -LiteralPath $current -Destination $backup -Force
  }}
  sc.exe stop $serviceName | Out-Null
  Start-Sleep -Seconds 2
  Copy-Item -LiteralPath $staged -Destination $current -Force
  sc.exe start $serviceName | Out-Null
  Start-Sleep -Seconds 2
  exit 0
}} catch {{
  try {{
    if (Test-Path -LiteralPath $backup) {{
      Copy-Item -LiteralPath $backup -Destination $current -Force
    }}
    sc.exe start $serviceName | Out-Null
  }} catch {{}}
  exit 1
}}
"#,
        service = ps_quote(&plan.service_name),
        current = ps_quote(&plan.current_exe.display().to_string()),
        staged = ps_quote(&staged_path.display().to_string()),
        backup = ps_quote(&plan.backup_path.display().to_string()),
    )
}

fn unix_update_script(plan: &UpdateSwitchPlan, staged_path: &Path) -> String {
    format!(
        r#"#!/bin/sh
set -eu
sleep 1
current={current}
staged={staged}
backup={backup}
service={service}
mkdir -p "$(dirname "$backup")"
if [ -f "$current" ]; then
  cp "$current" "$backup"
fi
if command -v systemctl >/dev/null 2>&1; then
  systemctl stop "$service" >/dev/null 2>&1 || true
fi
if cp "$staged" "$current"; then
  chmod 755 "$current" || true
  if command -v systemctl >/dev/null 2>&1; then
    systemctl start "$service" >/dev/null 2>&1 || true
  fi
  exit 0
fi
if [ -f "$backup" ]; then
  cp "$backup" "$current" || true
fi
if command -v systemctl >/dev/null 2>&1; then
  systemctl start "$service" >/dev/null 2>&1 || true
fi
exit 1
"#,
        current = sh_quote(&plan.current_exe.display().to_string()),
        staged = sh_quote(&staged_path.display().to_string()),
        backup = sh_quote(&plan.backup_path.display().to_string()),
        service = sh_quote(&plan.service_name),
    )
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn file_sha256_hex(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("failed to open update source {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buf)
            .with_context(|| format!("failed to read update source {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn binary_version(path: &Path) -> Result<String> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .with_context(|| format!("failed to run staged update candidate {}", path.display()))?;
    if !output.status.success() {
        anyhow::bail!(
            "staged update candidate --version failed with status {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let text = String::from_utf8(output.stdout)
        .context("staged update candidate --version output was not utf-8")?;
    let version = text
        .split_whitespace()
        .last()
        .ok_or_else(|| anyhow!("staged update candidate returned empty --version output"))?;
    Ok(version.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_has_stable_length() {
        let path =
            std::env::temp_dir().join(format!("ssh_proxy-update-hash-{}.bin", std::process::id()));
        std::fs::write(&path, b"update-candidate").unwrap();

        let hash = file_sha256_hex(&path).unwrap();

        assert_eq!(hash.len(), 64);
        assert_eq!(hash, file_sha256_hex(&path).unwrap());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn update_switch_script_is_allowlisted_to_service_replacement() {
        let dir =
            std::env::temp_dir().join(format!("ssh_proxy-update-script-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plan = UpdateSwitchPlan {
            service_name: "ssh_proxy".to_string(),
            current_exe: dir.join("ssh_proxy.exe"),
            backup_path: dir.join("ssh_proxy.backup.exe"),
            script_path: dir.join("update.ps1"),
        };
        let staged = dir.join("ssh_proxy.staged.exe");

        let script = windows_update_script(&plan, &staged);

        assert!(script.contains("sc.exe stop"));
        assert!(script.contains("Copy-Item -LiteralPath $staged -Destination $current -Force"));
        assert!(script.contains("Copy-Item -LiteralPath $backup -Destination $current -Force"));
        assert!(!script.contains("Invoke-Expression"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
