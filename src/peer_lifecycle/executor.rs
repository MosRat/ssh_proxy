use std::{future::Future, pin::Pin, process::Command};

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

    fn upload_bytes<'a>(&'a self, _command: String, _bytes: Vec<u8>) -> BoxExecutorFuture<'a, ()> {
        Box::pin(
            async move { bail!("LocalExecutor upload_bytes is intentionally not implemented") },
        )
    }
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
}
