use std::{fs, future::Future, path::Path, pin::Pin, process::Command};

use anyhow::{Context, Result, bail};

use crate::{ssh_client, ssh_client::ExecOutput};

pub(crate) type BoxExecutorFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

pub(crate) trait PeerExecutor {
    fn exec_capture<'a>(
        &'a self,
        command: String,
        stdin: Option<Vec<u8>>,
    ) -> BoxExecutorFuture<'a, ExecOutput>;

    fn upload_bytes<'a>(&'a self, command: String, bytes: Vec<u8>) -> BoxExecutorFuture<'a, ()>;
}

pub(crate) struct SshExecutor<'a> {
    client: &'a ssh_client::Client,
}

impl<'a> SshExecutor<'a> {
    pub(crate) fn new(client: &'a ssh_client::Client) -> Self {
        Self { client }
    }
}

impl PeerExecutor for SshExecutor<'_> {
    fn exec_capture<'a>(
        &'a self,
        command: String,
        stdin: Option<Vec<u8>>,
    ) -> BoxExecutorFuture<'a, ExecOutput> {
        Box::pin(async move { self.client.exec_capture(command, stdin).await })
    }

    fn upload_bytes<'a>(&'a self, command: String, bytes: Vec<u8>) -> BoxExecutorFuture<'a, ()> {
        Box::pin(async move { self.client.exec_upload(command, bytes).await })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct LocalExecutor;

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
}

pub(crate) fn write_local_file(path: &Path, bytes: &[u8], preserve_existing: bool) -> Result<()> {
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
#[derive(Debug, Default)]
pub(crate) struct FakeExecutor {
    pub(crate) outputs: std::sync::Mutex<Vec<ExecOutput>>,
    pub(crate) commands: std::sync::Mutex<Vec<String>>,
}

#[cfg(test)]
impl FakeExecutor {
    pub(crate) fn push_output(&self, output: ExecOutput) {
        self.outputs.lock().unwrap().push(output);
    }

    pub(crate) fn commands(&self) -> Vec<String> {
        self.commands.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl PeerExecutor for FakeExecutor {
    fn exec_capture<'a>(
        &'a self,
        command: String,
        _stdin: Option<Vec<u8>>,
    ) -> BoxExecutorFuture<'a, ExecOutput> {
        Box::pin(async move {
            self.commands.lock().unwrap().push(command);
            self.outputs
                .lock()
                .unwrap()
                .pop()
                .context("fake executor has no queued output")
        })
    }

    fn upload_bytes<'a>(&'a self, command: String, _bytes: Vec<u8>) -> BoxExecutorFuture<'a, ()> {
        Box::pin(async move {
            self.commands.lock().unwrap().push(command);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_executor_records_commands() {
        let executor = FakeExecutor::default();
        executor.push_output(ExecOutput {
            exit_status: 0,
            stdout: "ok".to_string(),
            stderr: String::new(),
        });

        let output = executor
            .exec_capture("echo ok".to_string(), None)
            .await
            .unwrap();

        assert_eq!(output.stdout, "ok");
        assert_eq!(executor.commands(), vec!["echo ok"]);
    }

    #[test]
    fn local_file_write_preserves_existing_when_requested() {
        let dir = std::env::temp_dir().join(format!(
            "ssh_proxy-local-executor-{}",
            std::process::id()
        ));
        let path = dir.join("config.toml");
        write_local_file(&path, b"first", false).unwrap();
        write_local_file(&path, b"second", true).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        let _ = std::fs::remove_dir_all(dir);
    }
}
