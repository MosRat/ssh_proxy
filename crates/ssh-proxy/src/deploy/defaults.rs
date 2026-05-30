use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use ssh_proxy_core::model::RemotePlatform;
use ssh_proxy_deploy::{RemoteAdminDefaultsReport, RemoteAdminIntent, remote_admin_stdin_command};
use tracing::info;

use crate::{cli, config, peer_lifecycle, ssh_client};

use super::remote_commands::remote_resolve_peer_defaults_command;
use peer_lifecycle::executor::PeerExecutor;

pub(super) async fn apply_remote_auto_defaults(
    client: &ssh_client::Client,
    args: &mut cli::InstallRemoteArgs,
) -> Result<()> {
    if args.remote_token.is_none() {
        args.remote_token = Some(config::generate_token()?);
    }
    let token = args.remote_token.as_deref().unwrap_or_default();
    let mut resolved_node_id = None;
    let mut resolved_node_name = None;
    if let Some(remote_path) = args.remote_path.as_deref() {
        match remote_defaults_via_admin(client, remote_path, args).await {
            Ok(report) => {
                if report.transport == report.control {
                    anyhow::bail!(
                        "remote admin defaults returned overlapping transport/control endpoint {}",
                        report.transport
                    );
                }
                args.remote_tcp = report.transport;
                args.remote_control = report.control;
                resolved_node_id = report.node_id;
                resolved_node_name = Some(report.node_name);
            }
            Err(err) => {
                info!(
                    remote_path,
                    error = %err,
                    "remote admin defaults failed; falling back to shell defaults probe"
                );
            }
        }
    }
    if resolved_node_name.is_none() {
        let output = client
            .exec_output(remote_resolve_peer_defaults_command(
                args.remote_tcp,
                args.remote_control,
                args.remote_os,
            ))
            .await?;
        for line in output.lines() {
            if let Some(value) = line.strip_prefix("transport=") {
                args.remote_tcp = value
                    .parse()
                    .with_context(|| format!("invalid remote transport address {value:?}"))?;
            } else if let Some(value) = line.strip_prefix("control=") {
                args.remote_control = value
                    .parse()
                    .with_context(|| format!("invalid remote control address {value:?}"))?;
            } else if let Some(value) = line.strip_prefix("node_id=") {
                if !value.trim().is_empty() {
                    resolved_node_id = Some(value.to_string());
                }
            } else if let Some(value) = line.strip_prefix("node_name=")
                && !value.trim().is_empty()
            {
                resolved_node_name = Some(value.to_string());
            }
        }
        if args.remote_tcp == args.remote_control {
            anyhow::bail!(
                "remote defaults returned overlapping transport/control endpoint {}",
                args.remote_tcp
            );
        }
    }
    let node_id = args
        .remote_node_id
        .clone()
        .or(resolved_node_id)
        .map(Ok)
        .unwrap_or_else(peer_lifecycle::spec::generated_remote_node_id)?;
    let node_name = args
        .remote_node_name
        .clone()
        .or(resolved_node_name)
        .unwrap_or_else(|| args.target.clone());
    let files =
        peer_lifecycle::config::materialize_peer_config(&peer_lifecycle::config::PeerConfigInput {
            node_id: node_id.clone(),
            node_name: node_name.clone(),
            token: token.to_string(),
            transport: args.remote_tcp,
            control: args.remote_control,
            local_node_id: args.local_node_id.clone(),
            local_node_name: args.local_node_name.clone(),
            local_control_endpoint: args.local_control_endpoint.clone(),
            local_transport: args.local_transport,
            service_manager: "pending".to_string(),
            updated_at_unix: now_unix(),
        });
    let store_bundle = peer_lifecycle::store::PeerStoreBundle::from_config_files(files)?;
    let executor = peer_lifecycle::executor::SshExecutor::new(client);
    for artifact in store_bundle.into_artifacts() {
        executor
            .write_artifact(
                peer_lifecycle::commands::remote_write_peer_artifact_command(
                    artifact.artifact,
                    args.remote_os,
                ),
                artifact.artifact,
                artifact.bytes,
            )
            .await
            .with_context(|| {
                format!(
                    "failed to write remote peer {}",
                    artifact.artifact.file_name()
                )
            })?;
    }
    args.remote_node_id = Some(node_id);
    args.remote_node_name = Some(node_name);
    Ok(())
}

async fn remote_defaults_via_admin(
    client: &ssh_client::Client,
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> Result<RemoteAdminDefaultsReport> {
    let remote_platform: RemotePlatform = args.remote_os.into();
    let command = remote_admin_stdin_command(remote_path, remote_platform);
    let intent = RemoteAdminIntent::Defaults {
        preferred_transport: args.remote_tcp,
        preferred_control: args.remote_control,
    };
    let stdin = serde_json::to_vec(&intent).context("failed to encode remote admin defaults")?;
    let output = client.exec_capture(command, Some(stdin)).await?;
    if output.exit_status != 0 {
        anyhow::bail!(
            "remote admin defaults exited with status {}: {}",
            output.exit_status,
            output.stderr.trim()
        );
    }
    let response: serde_json::Value = serde_json::from_str(&output.stdout)
        .context("remote admin defaults did not return JSON")?;
    if !response["ok"].as_bool().unwrap_or(false) {
        anyhow::bail!(
            "remote admin defaults failed: {}",
            response["error"].as_str().unwrap_or("unknown error")
        );
    }
    serde_json::from_value(response["data"].clone())
        .context("remote admin defaults JSON has invalid data")
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
