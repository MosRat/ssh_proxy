use std::net::SocketAddr;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::{cli, config, ssh_client};

use super::{
    descriptor::apply_descriptor_to_install_args,
    remote_commands::{default_persistent_remote_path, remote_node_control_command},
};

pub(crate) async fn rotate_remote_peer_token(
    mut args: cli::InstallRemoteArgs,
) -> Result<RemoteTokenRotateResult> {
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
        "token-rotate",
    );
    let output = client.exec_output(command).await?;
    let response: Value = serde_json::from_str(&output)
        .with_context(|| format!("failed to parse remote token rotation from {}", args.target))?;
    if response["ok"] != true {
        bail!("remote token rotation failed: {response}");
    }
    let new_token = response
        .get("token")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("remote token rotation did not return a token"))?
        .to_string();
    args.remote_token = Some(new_token.clone());
    let descriptor = match remote_node_control_command(
        &remote_path,
        args.remote_control,
        args.remote_token.as_deref(),
        "descriptor",
    ) {
        command => match client.exec_output(command).await {
            Ok(output) => serde_json::from_str::<Value>(&output).ok(),
            Err(_) => None,
        },
    };
    if let Some(descriptor) = &descriptor {
        apply_descriptor_to_install_args(descriptor, &mut args);
    }
    let token_metadata = response
        .get("token_metadata")
        .and_then(|value| serde_json::from_value(value.clone()).ok());
    Ok(RemoteTokenRotateResult {
        target: args.target,
        remote_path,
        remote_control: args.remote_control,
        remote_tcp: args.remote_tcp,
        remote_tls_transport: args.remote_tls_transport,
        remote_quic_transport: args.remote_quic_transport,
        remote_token: new_token,
        token_metadata,
        descriptor,
        response,
    })
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteTokenRotateResult {
    pub(crate) target: String,
    pub(crate) remote_path: String,
    pub(crate) remote_control: SocketAddr,
    pub(crate) remote_tcp: SocketAddr,
    pub(crate) remote_tls_transport: Option<SocketAddr>,
    pub(crate) remote_quic_transport: Option<SocketAddr>,
    pub(crate) remote_token: String,
    pub(crate) token_metadata: Option<config::TokenMetadata>,
    pub(crate) descriptor: Option<Value>,
    pub(crate) response: Value,
}
