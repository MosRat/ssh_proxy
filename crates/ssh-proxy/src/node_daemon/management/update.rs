use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, anyhow};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{
    node_daemon::{NodeManager, NodeRequest, jobs, response_line},
    paths,
};

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
    pub(in crate::node_daemon) async fn reconcile_daemon_update_job(&self) -> Result<()> {
        let update_state = self.state.daemon_value().await;
        let update_healthy = update_state
            .pointer("/update/state")
            .and_then(serde_json::Value::as_str)
            == Some("healthy")
            || update_state
                .get("update_state")
                .and_then(serde_json::Value::as_str)
                == Some("healthy");
        if !update_healthy {
            return Ok(());
        }
        let Some(job) = self.jobs.get("self-update:pending").await else {
            return Ok(());
        };
        if !matches!(
            job.state,
            jobs::JobState::Queued | jobs::JobState::Running | jobs::JobState::WaitingRetry
        ) {
            return Ok(());
        }
        let mut completed = job.transition(jobs::JobState::Healthy, jobs::JobPhase::Healthy, 100);
        completed.blocker = None;
        completed.next_action = None;
        completed.last_error = None;
        self.jobs
            .upsert(completed, "daemon self-update completed after restart")
            .await?;
        Ok(())
    }

    pub(in crate::node_daemon) async fn daemon_update(
        &self,
        request: NodeRequest,
    ) -> Result<String> {
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
