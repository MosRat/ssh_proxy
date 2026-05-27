use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Command,
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
        let update_state = self
            .state
            .record_daemon_update_state(
                "staged",
                Some(staged.source.display().to_string()),
                Some(staged.staged_path.display().to_string()),
                Some(staged.hash.clone()),
                Some(staged.version.clone()),
                None,
            )
            .await?;
        let job = self
            .jobs
            .upsert(
                jobs::JobRecord::new(job.id.clone(), "self_update")
                    .transition(jobs::JobState::WaitingRetry, jobs::JobPhase::RestartDaemon, 80)
                    .with_next_action("restart daemon service to switch to the staged binary"),
                "daemon self-update staged; waiting for supervised switch",
            )
            .await?;
        response_line(json!({
            "ok": true,
            "kind": "daemon_update",
            "daemon_api": "v0.3",
            "job": job.to_value(),
            "update_state": update_state,
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
        let status = self.status_value().await?;
        let job_values = self.jobs.jobs_value().await;
        let mut daemon_jobs = job_values.as_array().cloned().unwrap_or_default();
        daemon_jobs.extend(jobs::route_jobs_from_status(&status));
        response_line(json!({
            "ok": true,
            "kind": "jobs",
            "daemon_api": "v0.3",
            "jobs": daemon_jobs,
            "message": "daemon jobs are the stable v0.3 progress surface",
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
                "next_action": "service_ensure_or_session_daemon_restart",
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
    std::fs::create_dir_all(&updates_dir)
        .with_context(|| format!("failed to create update staging dir {}", updates_dir.display()))?;
    let short_hash = hash.get(..12).unwrap_or(&hash);
    let staged_path = updates_dir.join(format!("ssh_proxy-staged-{short_hash}{}", std::env::consts::EXE_SUFFIX));
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
        let path = std::env::temp_dir().join(format!("ssh_proxy-update-hash-{}.bin", std::process::id()));
        std::fs::write(&path, b"update-candidate").unwrap();

        let hash = file_sha256_hex(&path).unwrap();

        assert_eq!(hash.len(), 64);
        assert_eq!(hash, file_sha256_hex(&path).unwrap());
        let _ = std::fs::remove_file(path);
    }
}
