use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::time;
use tracing::{info, warn};

use crate::control_socket;

use super::{NodeManager, NodeRequest, NodeResponse};

pub(super) async fn run_control_server(
    endpoint: control_socket::ControlEndpoint,
    manager: Arc<NodeManager>,
) -> Result<()> {
    let listener = control_socket::ControlListener::bind(&endpoint).await?;
    let require_auth = endpoint.is_tcp();
    info!(%endpoint, "node control listener ready");
    loop {
        tokio::select! {
            accept = listener.accept() => {
                let stream = accept?;
                let manager = manager.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_control(stream, manager, require_auth).await {
                        warn!(error = %err, "node control request failed");
                    }
                });
            }
            _ = manager.shutdown_notified() => break,
        }
    }
    Ok(())
}

async fn handle_control(
    stream: control_socket::ControlStream,
    manager: Arc<NodeManager>,
    require_auth: bool,
) -> Result<()> {
    let mut reader = BufReader::new(stream);
    let mut command = Vec::new();
    let read = time::timeout(
        control_socket::CONTROL_IO_TIMEOUT,
        (&mut reader)
            .take((control_socket::MAX_CONTROL_REQUEST_BYTES + 1) as u64)
            .read_until(b'\n', &mut command),
    )
    .await
    .map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::TimedOut, "control request timed out")
    })??;
    let mut stream = reader.into_inner();
    if read > control_socket::MAX_CONTROL_REQUEST_BYTES
        || command.len() > control_socket::MAX_CONTROL_REQUEST_BYTES
    {
        let response = NodeResponse::error_line("bad_request", "control request too large");
        stream.write_all(response.as_bytes()).await?;
        stream.shutdown().await.ok();
        return Ok(());
    }
    let command = String::from_utf8(command).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid utf-8 control request",
        )
    })?;
    let response = match parse_request(&command) {
        Ok(request) => match authenticate_request(&manager, &request, require_auth) {
            Ok(()) => match dispatch_request(manager, request).await {
                Ok(response) => response,
                Err(err) => NodeResponse::error_line("request_failed", err.to_string()),
            },
            Err(err) => NodeResponse::error_line("unauthorized", err.to_string()),
        },
        Err(err) => NodeResponse::error_line("bad_request", err.to_string()),
    };
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await.ok();
    Ok(())
}

fn authenticate_request(
    manager: &NodeManager,
    request: &NodeRequest,
    require_auth: bool,
) -> Result<()> {
    if !require_auth {
        return Ok(());
    }
    let expected = manager.token_value();
    let Some(expected) = expected.as_deref() else {
        return Ok(());
    };
    let provided = request
        .auth_token
        .as_deref()
        .ok_or_else(|| anyhow!("node control token is required for TCP endpoints"))?;
    if token_matches(provided, expected) {
        Ok(())
    } else {
        Err(anyhow!("invalid node control token"))
    }
}

fn token_matches(provided: &str, expected: &str) -> bool {
    let provided = provided.as_bytes();
    let expected = expected.as_bytes();
    if provided.len() != expected.len() {
        return false;
    }
    provided
        .iter()
        .zip(expected.iter())
        .fold(0_u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}

async fn dispatch_request(manager: Arc<NodeManager>, request: NodeRequest) -> Result<String> {
    match request.cmd.to_ascii_lowercase().as_str() {
        "status" | "" => manager.status_json().await,
        "descriptor" | "describe" => manager.descriptor_json().await,
        "links" => manager.links_json().await,
        "shutdown" => manager.shutdown().await,
        "nodes" | "node_list" | "node-list" => manager.nodes_json().await,
        "jobs" | "job_list" | "job-list" => manager.jobs_json().await,
        "node_ensure" | "node-ensure" => manager.node_ensure(request).await,
        "node_start" | "node-start" => manager.node_start(request).await,
        "node_stop" | "node-stop" => manager.node_stop(request).await,
        "node_restart" | "node-restart" => manager.node_restart(request).await,
        "connect" => {
            let profile = request
                .profile
                .ok_or_else(|| anyhow!("connect requires a profile"))?;
            let message = manager.connect_profile(&profile).await?;
            NodeResponse::ok_message(message).to_line()
        }
        "disconnect" => {
            let profile = request
                .profile
                .ok_or_else(|| anyhow!("disconnect requires a profile"))?;
            let message = manager.disconnect_profile(&profile).await?;
            NodeResponse::ok_message(message).to_line()
        }
        "route_start" => manager.start_route(request).await,
        "route_plan" | "route-plan" => manager.handle_route_plan(request).await,
        "route_intent" => manager.handle_route_intent(request).await,
        "route_stop" => manager.stop_route(request).await,
        "route_restart" => manager.restart_route(request).await,
        "route_list" | "routes" => manager.route_list_json().await,
        "peer_list" | "peers" => manager.peers_json().await,
        "token_rotate" | "token-rotate" => manager.rotate_token().await,
        "peer_bootstrap" => manager.bootstrap_peer(request).await,
        "peer_ensure" | "peer-ensure" => manager.ensure_peer(request).await,
        "peer_update" | "peer-update" => manager.update_peer(request).await,
        "peer_refresh" => manager.refresh_peer(request).await,
        "peer_diff" | "peer-diff" => manager.diff_peer(request).await,
        "peer_reconcile" | "peer-reconcile" => manager.reconcile_peer(request).await,
        "peer_check_version" | "peer-check-version" => manager.check_peer_version(request).await,
        "peer_rotate_token" | "peer-rotate-token" => manager.rotate_peer_token(request).await,
        "peer_forget" => manager.forget_peer(request).await,
        "report" => manager.record_report(request).await,
        other => Ok(NodeResponse::error_line(
            "unknown_command",
            format!("unknown node command {other:?}"),
        )),
    }
}

fn parse_request(command: &str) -> Result<NodeRequest> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Ok(NodeRequest::command("status"));
    }
    if trimmed.starts_with('{') {
        let request: NodeRequest =
            serde_json::from_str(trimmed).context("failed to parse node JSON request")?;
        request.validate_compatible()?;
        return Ok(request);
    }
    let mut parts = trimmed.split_whitespace();
    let cmd = parts.next().unwrap_or("status").to_string();
    let profile = parts.next().map(ToOwned::to_owned);
    Ok(NodeRequest::legacy_command(cmd, profile))
}
