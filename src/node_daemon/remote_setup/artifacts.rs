use anyhow::{Context, Result};

use crate::{
    peer_lifecycle::{
        artifacts::PeerArtifact,
        executor::{PeerExecutor, SshExecutor},
    },
    ssh_client,
};

use super::shell::shell_quote;

pub(super) async fn read_remote_setup_artifact(
    client: &ssh_client::Client,
    server_dir: &str,
    relative_path: &str,
    label: &str,
) -> Result<String> {
    let command = build_remote_setup_read_command(server_dir, relative_path);
    let executor = SshExecutor::new(client);
    let bytes = executor
        .read_artifact(command)
        .await
        .with_context(|| format!("{label} failed"))?;
    String::from_utf8(bytes).with_context(|| format!("{label} returned non-UTF-8 content"))
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
    let command = build_remote_setup_write_command(server_dir, relative_path, backup_existing);
    let executor = SshExecutor::new(client);
    executor
        .write_artifact(command, artifact, bytes)
        .await
        .with_context(|| format!("{label} failed"))
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
