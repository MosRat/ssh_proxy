use std::net::SocketAddr;

use serde_json::{Value, json};

use crate::{cli, config, deploy};

pub(super) fn install_args_from_bootstrap(args: &cli::PeerBootstrapArgs) -> cli::InstallRemoteArgs {
    cli::InstallRemoteArgs {
        target: args.target.clone(),
        ssh_args: args.ssh_args.clone(),
        ssh_command: None,
        user: args.user.clone(),
        port: args.port,
        identity: args.identity.clone(),
        config: args.config.clone(),
        known_hosts: args.known_hosts.clone(),
        accept_new: args.accept_new,
        insecure_ignore_host_key: args.insecure_ignore_host_key,
        jump: args.jump.clone(),
        remote_path: args.remote_path.clone(),
        remote_bin: args.remote_bin.clone(),
        remote_os: args.remote_os,
        remote_token: args.remote_token.clone(),
        remote_tcp: args.remote_tcp,
        remote_control: args.remote_control,
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

pub(super) fn install_args_for_bootstrap(
    args: &cli::PeerBootstrapArgs,
    identity: config::NodeIdentity,
    local_control_endpoint: String,
    local_transport: Option<SocketAddr>,
) -> cli::InstallRemoteArgs {
    let mut install_args = install_args_from_bootstrap(args);
    install_args.local_node_id = identity.node_id;
    install_args.local_node_name = identity.node_name;
    install_args.local_control_endpoint = Some(local_control_endpoint);
    install_args.local_transport = local_transport;
    install_args.persist = cli::PersistMode::Auto;
    install_args
}

pub(super) fn bootstrap_response(alias: &str, result: &deploy::RemoteInstallResult) -> Value {
    json!({
        "ok": true,
        "message": format!("peer {alias:?} bootstrapped"),
        "alias": alias,
        "target": result.target,
        "node_id": result.remote_node_id,
        "node_name": result.remote_node_name,
        "remote_path": result.remote_path,
        "remote_tcp": result.remote_tcp.to_string(),
        "remote_control": result.remote_control.to_string(),
        "remote_tls_transport": result.remote_tls_transport.map(|addr| addr.to_string()),
        "remote_quic_transport": result.remote_quic_transport.map(|addr| addr.to_string()),
        "changed": true
    })
}

pub(super) fn refresh_response(alias: &str, result: &deploy::RemoteDescriptorResult) -> Value {
    json!({
        "ok": true,
        "message": format!("peer {alias:?} refreshed"),
        "alias": alias,
        "target": result.target,
        "node_id": result.descriptor.get("node_id").cloned().unwrap_or(Value::Null),
        "node_name": result.descriptor.get("node_name").cloned().unwrap_or(Value::Null),
        "version": result.descriptor.get("version").cloned().unwrap_or(Value::Null),
        "control_api_version": result.descriptor.get("control_api_version").cloned().unwrap_or(Value::Null),
        "peer_protocol_version": result.descriptor.get("peer_protocol_version").cloned().unwrap_or(Value::Null),
        "transport_protocols": result.descriptor.get("transport_protocols").cloned().unwrap_or(Value::Null),
        "changed": true
    })
}

pub(super) fn token_rotation_response(
    alias: &str,
    result: &deploy::RemoteTokenRotateResult,
) -> Value {
    json!({
        "ok": true,
        "message": format!("peer {alias:?} token rotated"),
        "alias": alias,
        "target": result.target,
        "node_id": result
            .descriptor
            .as_ref()
            .and_then(|descriptor| descriptor.get("node_id"))
            .cloned()
            .unwrap_or(Value::Null),
        "node_name": result
            .descriptor
            .as_ref()
            .and_then(|descriptor| descriptor.get("node_name"))
            .cloned()
            .unwrap_or(Value::Null),
        "token_metadata": result.token_metadata,
        "remote_response": result.response,
        "changed": true
    })
}
