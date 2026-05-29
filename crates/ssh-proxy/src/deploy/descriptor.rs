use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::time::{self, Instant};

use crate::{cli, protocol_core::descriptor::PeerDescriptor, ssh_client};

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
    match PeerDescriptor::from_value(descriptor.clone()) {
        Ok(descriptor) => apply_peer_descriptor_to_install_args(&descriptor, args),
        Err(_) => apply_legacy_descriptor_to_install_args(descriptor, args),
    }
}

fn apply_peer_descriptor_to_install_args(
    descriptor: &PeerDescriptor,
    args: &mut cli::InstallRemoteArgs,
) {
    if let Some(control) = descriptor.control_addr() {
        args.remote_control = control;
    }
    if let Some(transport) = descriptor.transport_addr() {
        args.remote_tcp = transport;
    }
    if let Some(transport) = descriptor.tls_transport_addr() {
        args.remote_tls_transport = Some(transport);
    }
    if let Some(transport) = descriptor.quic_transport_addr() {
        args.remote_quic_transport = Some(transport);
    }
}

fn apply_legacy_descriptor_to_install_args(descriptor: &Value, args: &mut cli::InstallRemoteArgs) {
    if let Some(control) = descriptor
        .pointer("/endpoints/control")
        .and_then(Value::as_str)
        .and_then(parse_socket_or_tcp_endpoint)
    {
        args.remote_control = control;
    }
    if let Some(transport) = descriptor
        .pointer("/endpoints/transport")
        .and_then(Value::as_str)
        .and_then(parse_socket_or_tcp_endpoint)
    {
        args.remote_tcp = transport;
    }
    if let Some(transport) = descriptor
        .pointer("/endpoints/tls_transport")
        .and_then(Value::as_str)
        .and_then(parse_socket_or_tcp_endpoint)
    {
        args.remote_tls_transport = Some(transport);
    }
    if let Some(transport) = descriptor
        .pointer("/endpoints/quic_transport")
        .and_then(Value::as_str)
        .and_then(parse_socket_or_tcp_endpoint)
    {
        args.remote_quic_transport = Some(transport);
    }
}

fn parse_socket_or_tcp_endpoint(value: &str) -> Option<SocketAddr> {
    value.strip_prefix("tcp://").unwrap_or(value).parse().ok()
}

pub(super) fn descriptor_protocols(descriptor: &Value) -> Option<Vec<String>> {
    let protocols = PeerDescriptor::from_value(descriptor.clone())
        .map(|descriptor| descriptor.transport_protocols_or_infer())
        .unwrap_or_else(|_| {
            descriptor
                .get("transport_protocols")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        });
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

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use serde_json::json;

    use super::*;

    fn install_args() -> cli::InstallRemoteArgs {
        cli::InstallRemoteArgs {
            target: "remote".to_string(),
            ssh_args: Vec::new(),
            ssh_command: None,
            user: None,
            port: None,
            identity: Vec::new(),
            config: None,
            known_hosts: None,
            accept_new: false,
            insecure_ignore_host_key: false,
            jump: Vec::new(),
            remote_path: None,
            remote_bin: None,
            remote_os: cli::RemoteOs::Auto,
            remote_token: None,
            remote_tcp: "127.0.0.1:19080".parse().unwrap(),
            remote_control: "127.0.0.1:19081".parse().unwrap(),
            local_node_id: None,
            local_node_name: None,
            local_control_endpoint: None,
            local_transport: None,
            remote_node_id: None,
            remote_node_name: None,
            remote_tls_transport: None,
            remote_quic_transport: None,
            remote_tls_cert: None,
            remote_tls_key: None,
            remote_tls_client_ca: None,
            persist: cli::PersistMode::None,
        }
    }

    #[test]
    fn descriptor_updates_install_args_through_typed_dto() {
        let descriptor = json!({
            "ok": true,
            "endpoints": {
                "control": "tcp://127.0.0.1:29081",
                "transport": "127.0.0.1:29080",
                "tls_transport": "127.0.0.1:29443",
                "quic_transport": "127.0.0.1:29444"
            }
        });
        let mut args = install_args();

        apply_descriptor_to_install_args(&descriptor, &mut args);

        assert_eq!(
            args.remote_control,
            "127.0.0.1:29081".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            args.remote_tcp,
            "127.0.0.1:29080".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            args.remote_tls_transport,
            Some("127.0.0.1:29443".parse().unwrap())
        );
        assert_eq!(
            args.remote_quic_transport,
            Some("127.0.0.1:29444".parse().unwrap())
        );
    }

    #[test]
    fn descriptor_protocols_use_typed_inference() {
        let descriptor = json!({
            "ok": true,
            "endpoints": {
                "transport": "127.0.0.1:19080",
                "tls_transport": "127.0.0.1:19443"
            }
        });

        assert_eq!(
            descriptor_protocols(&descriptor),
            Some(vec!["tls-tcp".to_string(), "plain-tcp".to_string()])
        );
    }
}
