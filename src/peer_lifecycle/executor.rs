use std::{fs, future::Future, net::SocketAddr, path::Path, pin::Pin, process::Command};

use anyhow::{Context, Result, bail};
use tokio::net::TcpStream;

use crate::{peer_lifecycle::artifacts::PeerArtifact, ssh_client, ssh_client::ExecOutput};

pub(crate) type BoxExecutorFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceControlAction {
    Install,
    Start,
    Stop,
    Status,
    Rollback,
}

impl ServiceControlAction {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Status => "status",
            Self::Rollback => "rollback",
        }
    }
}

pub(crate) trait PeerExecutor {
    fn exec_capture<'a>(
        &'a self,
        command: String,
        stdin: Option<Vec<u8>>,
    ) -> BoxExecutorFuture<'a, ExecOutput>;

    fn upload_bytes<'a>(&'a self, command: String, bytes: Vec<u8>) -> BoxExecutorFuture<'a, ()>;

    fn write_artifact<'a>(
        &'a self,
        target: String,
        artifact: PeerArtifact,
        bytes: Vec<u8>,
    ) -> BoxExecutorFuture<'a, ()> {
        let _ = artifact;
        self.upload_bytes(target, bytes)
    }

    fn read_artifact<'a>(&'a self, target: String) -> BoxExecutorFuture<'a, Vec<u8>> {
        Box::pin(
            async move { bail!("reading lifecycle artifact is not supported for target {target}") },
        )
    }

    fn stage_binary<'a>(&'a self, source: String, target: String) -> BoxExecutorFuture<'a, ()> {
        Box::pin(async move {
            bail!("binary staging from {source} to {target} is not supported by this executor")
        })
    }

    fn probe_tcp<'a>(&'a self, addr: SocketAddr) -> BoxExecutorFuture<'a, ()> {
        Box::pin(async move {
            TcpStream::connect(addr)
                .await
                .with_context(|| format!("failed to probe TCP endpoint {addr}"))?;
            Ok(())
        })
    }

    fn service_control<'a>(
        &'a self,
        service_name: String,
        action: ServiceControlAction,
    ) -> BoxExecutorFuture<'a, ExecOutput> {
        Box::pin(async move {
            bail!(
                "service control {} for {service_name} is not supported by this executor",
                action.as_str()
            )
        })
    }
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

    fn probe_tcp<'a>(&'a self, addr: SocketAddr) -> BoxExecutorFuture<'a, ()> {
        Box::pin(async move {
            let _stream = self
                .client
                .direct_tcpip_stream(addr.ip().to_string(), addr.port())
                .await
                .with_context(|| format!("failed to probe remote TCP endpoint {addr}"))?;
            Ok(())
        })
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
    pub(crate) artifacts: std::sync::Mutex<Vec<(String, PeerArtifact, Vec<u8>)>>,
    pub(crate) service_controls: std::sync::Mutex<Vec<(String, ServiceControlAction)>>,
}

#[cfg(test)]
impl FakeExecutor {
    pub(crate) fn push_output(&self, output: ExecOutput) {
        self.outputs.lock().unwrap().push(output);
    }

    pub(crate) fn commands(&self) -> Vec<String> {
        self.commands.lock().unwrap().clone()
    }

    pub(crate) fn artifacts(&self) -> Vec<(String, PeerArtifact, Vec<u8>)> {
        self.artifacts.lock().unwrap().clone()
    }

    pub(crate) fn service_controls(&self) -> Vec<(String, ServiceControlAction)> {
        self.service_controls.lock().unwrap().clone()
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

    fn write_artifact<'a>(
        &'a self,
        target: String,
        artifact: PeerArtifact,
        bytes: Vec<u8>,
    ) -> BoxExecutorFuture<'a, ()> {
        Box::pin(async move {
            self.artifacts
                .lock()
                .unwrap()
                .push((target, artifact, bytes));
            Ok(())
        })
    }

    fn stage_binary<'a>(&'a self, source: String, target: String) -> BoxExecutorFuture<'a, ()> {
        Box::pin(async move {
            self.commands
                .lock()
                .unwrap()
                .push(format!("stage_binary {source} {target}"));
            Ok(())
        })
    }

    fn service_control<'a>(
        &'a self,
        service_name: String,
        action: ServiceControlAction,
    ) -> BoxExecutorFuture<'a, ExecOutput> {
        Box::pin(async move {
            self.service_controls
                .lock()
                .unwrap()
                .push((service_name, action));
            self.outputs
                .lock()
                .unwrap()
                .pop()
                .context("fake executor has no queued service control output")
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
        let dir =
            std::env::temp_dir().join(format!("ssh_proxy-local-executor-{}", std::process::id()));
        let path = dir.join("config.toml");
        write_local_file(&path, b"first", false).unwrap();
        write_local_file(&path, b"second", true).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn fake_executor_records_structured_artifacts_and_stage() {
        let executor = FakeExecutor::default();

        executor
            .write_artifact(
                "config.toml".to_string(),
                PeerArtifact::Config,
                b"config".to_vec(),
            )
            .await
            .unwrap();
        executor
            .stage_binary("source".to_string(), "target".to_string())
            .await
            .unwrap();

        assert_eq!(executor.artifacts()[0].0, "config.toml");
        assert_eq!(executor.artifacts()[0].1, PeerArtifact::Config);
        assert_eq!(executor.commands(), vec!["stage_binary source target"]);
    }

    #[tokio::test]
    async fn fake_executor_records_service_control() {
        let executor = FakeExecutor::default();
        executor.push_output(ExecOutput {
            exit_status: 0,
            stdout: "running".to_string(),
            stderr: String::new(),
        });

        let output = executor
            .service_control("ssh_proxy".to_string(), ServiceControlAction::Status)
            .await
            .unwrap();

        assert_eq!(output.stdout, "running");
        assert_eq!(
            executor.service_controls(),
            vec![("ssh_proxy".to_string(), ServiceControlAction::Status)]
        );
    }
}
