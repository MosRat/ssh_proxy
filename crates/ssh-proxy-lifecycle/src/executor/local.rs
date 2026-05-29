use std::{fs, path::Path, process::Command};

use anyhow::{Context, Result, bail};

use crate::artifacts::PeerArtifact;
use ssh_proxy_core::command::ExecOutput;

use super::model::{BoxExecutorFuture, PeerExecutor, ServiceControlAction};

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalExecutor;

impl PeerExecutor for LocalExecutor {
    fn exec_capture<'a>(
        &'a self,
        command: String,
        stdin: Option<Vec<u8>>,
    ) -> BoxExecutorFuture<'a, ExecOutput> {
        Box::pin(async move {
            if stdin.is_some() {
                bail!("LocalExecutor does not support stdin for shell commands yet");
            }
            let output = if cfg!(windows) {
                Command::new("cmd")
                    .args(["/C", &command])
                    .output()
                    .with_context(|| format!("failed to run local command {command}"))?
            } else {
                Command::new("sh")
                    .args(["-c", &command])
                    .output()
                    .with_context(|| format!("failed to run local command {command}"))?
            };
            Ok(ExecOutput {
                exit_status: output.status.code().unwrap_or(1) as u32,
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        })
    }

    fn upload_bytes<'a>(&'a self, path: String, bytes: Vec<u8>) -> BoxExecutorFuture<'a, ()> {
        Box::pin(async move { write_local_file(Path::new(&path), &bytes, false) })
    }

    fn write_artifact<'a>(
        &'a self,
        path: String,
        artifact: PeerArtifact,
        bytes: Vec<u8>,
    ) -> BoxExecutorFuture<'a, ()> {
        Box::pin(
            async move { write_local_file(Path::new(&path), &bytes, artifact.preserve_existing()) },
        )
    }

    fn read_artifact<'a>(&'a self, path: String) -> BoxExecutorFuture<'a, Vec<u8>> {
        Box::pin(async move {
            fs::read(&path).with_context(|| format!("failed to read lifecycle artifact {path}"))
        })
    }

    fn stage_binary<'a>(&'a self, source: String, target: String) -> BoxExecutorFuture<'a, ()> {
        Box::pin(async move {
            if source == target {
                return Ok(());
            }
            if let Some(parent) = Path::new(&target).parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::copy(&source, &target)
                .with_context(|| format!("failed to stage binary from {source} to {target}"))?;
            Ok(())
        })
    }

    fn service_control<'a>(
        &'a self,
        service_name: String,
        action: ServiceControlAction,
    ) -> BoxExecutorFuture<'a, ExecOutput> {
        Box::pin(async move {
            Ok(ExecOutput {
                exit_status: 1,
                stdout: String::new(),
                stderr: format!(
                    "local service control {} for {service_name} is not implemented in LocalExecutor",
                    action.as_str()
                ),
            })
        })
    }
}

pub fn write_local_file(path: &Path, bytes: &[u8], preserve_existing: bool) -> Result<()> {
    if preserve_existing && path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("file")
    ));
    fs::write(&tmp, bytes).with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| {
        format!(
            "failed to atomically replace {} with {}",
            path.display(),
            tmp.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_file_write_preserves_existing_when_requested() {
        let dir =
            std::env::temp_dir().join(format!("ssh_proxy-local-executor-{}", std::process::id()));
        let path = dir.join("config.toml");
        write_local_file(&path, b"first", false).unwrap();
        write_local_file(&path, b"second", true).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        let _ = std::fs::remove_dir_all(dir);
    }
}
