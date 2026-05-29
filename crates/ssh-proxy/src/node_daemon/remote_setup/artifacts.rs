use anyhow::{Context, Result};

use crate::{
    peer_lifecycle::{
        artifacts::PeerArtifact,
        executor::{PeerExecutor, SshExecutor},
    },
    ssh_client,
};

use super::shell::shell_quote;

#[derive(Debug, Clone)]
pub(super) struct RemoteArtifactPlan {
    server_dir: String,
    relative_path: String,
    artifact: PeerArtifact,
    backup_existing: bool,
    label: String,
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
            server_dir: server_dir.to_string(),
            relative_path: relative_path.to_string(),
            artifact,
            backup_existing,
            label: label.to_string(),
        }
    }

    pub(super) fn read_command(&self) -> String {
        build_remote_setup_read_command(&self.server_dir, &self.relative_path)
    }

    pub(super) fn write_command(&self) -> String {
        build_remote_setup_write_command(
            &self.server_dir,
            &self.relative_path,
            self.backup_existing,
        )
    }

    pub(super) async fn read(&self, client: &ssh_client::Client) -> Result<String> {
        let executor = SshExecutor::new(client);
        let bytes = executor
            .read_artifact(self.read_command())
            .await
            .with_context(|| format!("{} failed", self.label))?;
        String::from_utf8(bytes)
            .with_context(|| format!("{} returned non-UTF-8 content", self.label))
    }

    pub(super) async fn write(&self, client: &ssh_client::Client, bytes: Vec<u8>) -> Result<()> {
        let executor = SshExecutor::new(client);
        executor
            .write_artifact(self.write_command(), self.artifact, bytes)
            .await
            .with_context(|| format!("{} failed", self.label))
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
        PeerArtifact::VscodeRemoteStatus,
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

pub(super) fn build_remote_setup_read_command(server_dir: &str, relative_path: &str) -> String {
    format!(
        "set -eu; server_dir={server_dir}; relative_path={relative_path}; target=\"$HOME/$server_dir/$relative_path\"; if [ -f \"$target\" ]; then cat \"$target\"; fi",
        server_dir = shell_quote(server_dir),
        relative_path = shell_quote(relative_path),
    )
}

pub(super) fn build_remote_setup_write_command(
    server_dir: &str,
    relative_path: &str,
    backup_existing: bool,
) -> String {
    let backup = if backup_existing {
        "if [ -f \"$target\" ]; then cp \"$target\" \"$target.vscode-remote-proxy.bak\" 2>/dev/null || true; fi; "
    } else {
        ""
    };
    format!(
        "set -eu; server_dir={server_dir}; relative_path={relative_path}; target=\"$HOME/$server_dir/$relative_path\"; mkdir -p \"$(dirname \"$target\")\"; tmp=\"$target.tmp.$$\"; umask 077; cat > \"$tmp\"; {backup}mv \"$tmp\" \"$target\"; chmod 600 \"$target\" 2>/dev/null || true",
        server_dir = shell_quote(server_dir),
        relative_path = shell_quote(relative_path),
    )
}
