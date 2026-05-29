use anyhow::Context;

use crate::{peer_lifecycle::artifacts::PeerArtifact, ssh_client::ExecOutput};

use super::model::{BoxExecutorFuture, PeerExecutor, ServiceControlAction};

#[derive(Debug, Default)]
pub(crate) struct FakeExecutor {
    pub(crate) outputs: std::sync::Mutex<Vec<ExecOutput>>,
    pub(crate) commands: std::sync::Mutex<Vec<String>>,
    pub(crate) artifacts: std::sync::Mutex<Vec<(String, PeerArtifact, Vec<u8>)>>,
    pub(crate) service_controls: std::sync::Mutex<Vec<(String, ServiceControlAction)>>,
}

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
