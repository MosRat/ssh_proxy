use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::time::{self, Instant};

use crate::{cli, ssh_client};

use super::remote_commands::{default_persistent_remote_path, remote_node_control_command};

pub(crate) async fn refresh_remote_peer_descriptor(
    mut args: cli::InstallRemoteArgs,
) -> Result<RemoteDescriptorResult> {
    let client = ssh_client::Client::connect_install_args(&args).await?;
    let remote_path = if let Some(path) = args.remote_path.clone() {
        path
    } else {
        default_persistent_remote_path(&client, args.remote_os).await?
    };
    let command = remote_node_control_command(
        &remote_path,
        args.remote_control,
        args.remote_token.as_deref(),
        "descriptor",
    );
    let output = client.exec_output(command).await?;
    let descriptor: Value = serde_json::from_str(&output)
        .with_context(|| format!("failed to parse remote descriptor from {}", args.target))?;
    if descriptor["ok"] != true {
        bail!("remote descriptor request failed: {descriptor}");
    }
    apply_descriptor_to_install_args(&descriptor, &mut args);
    Ok(RemoteDescriptorResult {
        target: args.target,
        remote_path,
        remote_control: args.remote_control,
        remote_tcp: args.remote_tcp,
        remote_tls_transport: args.remote_tls_transport,
        remote_quic_transport: args.remote_quic_transport,
        remote_token: args.remote_token,
        descriptor,
    })
}

pub(super) async fn wait_remote_peer_descriptor(
    client: &ssh_client::Client,
    remote_path: &str,
    args: &mut cli::InstallRemoteArgs,
) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let command = remote_node_control_command(
            remote_path,
            args.remote_control,
            args.remote_token.as_deref(),
            "descriptor",
        );
        let last_error = match client.exec_output(command).await {
            Ok(output) => match serde_json::from_str::<Value>(&output) {
                Ok(descriptor) if descriptor["ok"] == true => {
                    apply_descriptor_to_install_args(&descriptor, args);
                    return Ok(descriptor);
                }
                Ok(descriptor) => Some(format!("remote descriptor returned not ok: {descriptor}")),
                Err(err) => Some(format!("failed to parse remote descriptor: {err}")),
            },
            Err(err) => Some(err.to_string()),
        };

        if Instant::now() >= deadline {
            bail!(
                "remote peer service did not become ready after install: {}",
                last_error.unwrap_or_else(|| "descriptor unavailable".to_string())
            );
        }
        time::sleep(Duration::from_millis(500)).await;
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteDescriptorResult {
    pub(crate) target: String,
    pub(crate) remote_path: String,
    pub(crate) remote_control: SocketAddr,
    pub(crate) remote_tcp: SocketAddr,
    pub(crate) remote_tls_transport: Option<SocketAddr>,
    pub(crate) remote_quic_transport: Option<SocketAddr>,
    pub(crate) remote_token: Option<String>,
    pub(crate) descriptor: Value,
}

pub(super) fn apply_descriptor_to_install_args(
    descriptor: &Value,
    args: &mut cli::InstallRemoteArgs,
) {
    if let Some(control) = descriptor
        .pointer("/endpoints/control")
        .and_then(Value::as_str)
        .and_then(parse_tcp_endpoint)
    {
        args.remote_control = control;
    }
    if let Some(transport) = descriptor
        .pointer("/endpoints/transport")
        .and_then(Value::as_str)
        .and_then(parse_socket_addr)
    {
        args.remote_tcp = transport;
    }
    if let Some(transport) = descriptor
        .pointer("/endpoints/tls_transport")
        .and_then(Value::as_str)
        .and_then(parse_socket_addr)
    {
        args.remote_tls_transport = Some(transport);
    }
    if let Some(transport) = descriptor
        .pointer("/endpoints/quic_transport")
        .and_then(Value::as_str)
        .and_then(parse_socket_addr)
    {
        args.remote_quic_transport = Some(transport);
    }
}

fn parse_tcp_endpoint(value: &str) -> Option<SocketAddr> {
    value.strip_prefix("tcp://").unwrap_or(value).parse().ok()
}

fn parse_socket_addr(value: &str) -> Option<SocketAddr> {
    value.parse().ok()
}

pub(super) fn descriptor_protocols(descriptor: &Value) -> Option<Vec<String>> {
    let protocols = descriptor
        .get("transport_protocols")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    (!protocols.is_empty()).then_some(protocols)
}

pub(super) fn descriptor_string_field(descriptor: &Value, field: &str) -> Option<String> {
    descriptor
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub(super) fn descriptor_u16_field(descriptor: &Value, field: &str) -> Option<u16> {
    descriptor
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
}

pub(super) fn descriptor_string_array_field(descriptor: &Value, field: &str) -> Vec<String> {
    descriptor
        .get(field)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn remote_descriptor_protocols(result: &RemoteDescriptorResult) -> Vec<String> {
    let mut protocols = Vec::new();
    if result.remote_quic_transport.is_some() {
        protocols.push("quic".to_string());
    }
    if result.remote_tls_transport.is_some() {
        protocols.push("tls-tcp".to_string());
    }
    protocols.push("plain-tcp".to_string());
    protocols
}
