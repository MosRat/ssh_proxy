use anyhow::{Context, Result};
use ssh_proxy_deploy::{RemoteArtifactIntent, RemoteArtifactKind};

use crate::{
    peer_lifecycle::{
        artifacts::PeerArtifact,
        executor::{PeerExecutor, SshExecutor},
    },
    ssh_client,
};

#[derive(Debug, Clone)]
pub(super) struct RemoteArtifactPlan {
    intent: RemoteArtifactIntent,
    artifact: PeerArtifact,
}

impl RemoteArtifactPlan {
    pub(super) fn new(
        server_dir: &str,
        relative_path: &str,
        artifact: PeerArtifact,
        backup_existing: bool,
        label: &str,
    ) -> Self {
        Self {
            intent: RemoteArtifactIntent::new(
                server_dir,
                relative_path,
                remote_artifact_kind(artifact),
                backup_existing,
                label,
            ),
            artifact,
        }
    }

    pub(super) fn read_command(&self) -> String {
        self.intent.read_command()
    }

    pub(super) fn write_command(&self) -> String {
        self.intent.write_command()
    }

    pub(super) async fn read(&self, client: &ssh_client::Client) -> Result<String> {
        let executor = SshExecutor::new(client);
        let bytes = executor
            .read_artifact(self.read_command())
            .await
            .with_context(|| format!("{} failed", self.intent.label))?;
        String::from_utf8(bytes)
            .with_context(|| format!("{} returned non-UTF-8 content", self.intent.label))
    }

    pub(super) async fn write(&self, client: &ssh_client::Client, bytes: Vec<u8>) -> Result<()> {
        let executor = SshExecutor::new(client);
        executor
            .write_artifact(self.write_command(), self.artifact, bytes)
            .await
            .with_context(|| format!("{} failed", self.intent.label))
    }
}

pub(super) async fn read_remote_setup_artifact(
    client: &ssh_client::Client,
    server_dir: &str,
    relative_path: &str,
    label: &str,
) -> Result<String> {
    RemoteArtifactPlan::new(
        server_dir,
        relative_path,
        peer_artifact_for_remote_path(relative_path),
        false,
        label,
    )
    .read(client)
    .await
}

pub(super) async fn write_remote_setup_artifact(
    client: &ssh_client::Client,
    server_dir: &str,
    relative_path: &str,
    artifact: PeerArtifact,
    bytes: Vec<u8>,
    backup_existing: bool,
    label: &str,
) -> Result<()> {
    RemoteArtifactPlan::new(server_dir, relative_path, artifact, backup_existing, label)
        .write(client, bytes)
        .await
}

fn remote_artifact_kind(artifact: PeerArtifact) -> RemoteArtifactKind {
    match artifact {
        PeerArtifact::VscodeMachineSettings => RemoteArtifactKind::VscodeMachineSettings,
        PeerArtifact::VscodeServerEnv => RemoteArtifactKind::VscodeServerEnv,
        PeerArtifact::VscodeRemoteStatus => RemoteArtifactKind::VscodeRemoteStatus,
        _ => unreachable!("remote setup only writes VS Code setup artifacts"),
    }
}

fn peer_artifact_for_remote_path(relative_path: &str) -> PeerArtifact {
    match relative_path {
        "data/Machine/settings.json" => PeerArtifact::VscodeMachineSettings,
        "server-env-setup" => PeerArtifact::VscodeServerEnv,
        "remote-proxy-status.json" => PeerArtifact::VscodeRemoteStatus,
        _ => PeerArtifact::VscodeRemoteStatus,
    }
}
