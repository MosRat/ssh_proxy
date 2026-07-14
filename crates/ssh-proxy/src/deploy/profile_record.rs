use anyhow::Result;
use serde_json::Value;

use crate::{config, node_daemon};

use super::{
    descriptor::{
        RemoteDescriptorResult, descriptor_protocols, descriptor_string_array_field,
        descriptor_string_field, descriptor_u16_field, remote_descriptor_protocols,
    },
    install::RemoteInstallResult,
    token::RemoteTokenRotateResult,
};

pub(crate) fn record_remote_token_rotation_profile(
    config: &mut config::AppConfig,
    profile_name: &str,
    result: &RemoteTokenRotateResult,
) -> Result<()> {
    apply_remote_token_rotation_profile(config, profile_name, result);
    config.save_default()
}

pub(super) fn apply_remote_token_rotation_profile(
    config: &mut config::AppConfig,
    profile_name: &str,
    result: &RemoteTokenRotateResult,
) {
    let profile = config.profiles.entry(profile_name.to_string()).or_default();
    if profile.target.is_none() {
        profile.target = Some(result.target.clone());
    }
    profile.remote_path = Some(result.remote_path.clone());
    profile.remote_control = Some(result.remote_control);
    profile.remote_tcp = Some(result.remote_tcp);
    profile.remote_tls = result.remote_tls_transport;
    profile.remote_quic = result.remote_quic_transport;
    profile.remote_transport = Some("auto".to_string());
    profile.remote_token = Some(result.remote_token.clone());

    let existing = config.peers.get(profile_name).cloned().unwrap_or_default();
    let descriptor = result.descriptor.as_ref();
    let node_id = descriptor
        .and_then(|descriptor| descriptor.get("node_id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or(existing.node_id);
    let node_name = descriptor
        .and_then(|descriptor| descriptor.get("node_name"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or(existing.node_name);
    let version = descriptor
        .and_then(|descriptor| descriptor_string_field(descriptor, "version"))
        .or(existing.version);
    let control_api_version = descriptor
        .and_then(|descriptor| descriptor_u16_field(descriptor, "control_api_version"))
        .or(existing.control_api_version);
    let peer_protocol_version = descriptor
        .and_then(|descriptor| descriptor_u16_field(descriptor, "peer_protocol_version"))
        .or(existing.peer_protocol_version);
    let features = descriptor
        .map(|descriptor| descriptor_string_array_field(descriptor, "features"))
        .filter(|features| !features.is_empty())
        .unwrap_or(existing.features);
    let os = descriptor
        .and_then(|descriptor| descriptor_string_field(descriptor, "os"))
        .or(existing.os);
    let arch = descriptor
        .and_then(|descriptor| descriptor_string_field(descriptor, "arch"))
        .or(existing.arch);
    let control_endpoint = descriptor
        .and_then(|descriptor| descriptor.pointer("/endpoints/control"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or(existing.control_endpoint)
        .unwrap_or_else(|| format!("tcp://{}", result.remote_control));
    let transport_protocols = descriptor
        .and_then(descriptor_protocols)
        .unwrap_or_else(|| {
            let mut protocols = Vec::new();
            if result.remote_quic_transport.is_some() {
                protocols.push("quic".to_string());
            }
            if result.remote_tls_transport.is_some() {
                protocols.push("tls-tcp".to_string());
            }
            protocols.push("plain-tcp".to_string());
            protocols
        });
    let token_generation = existing
        .token_metadata
        .as_ref()
        .map(|metadata| metadata.generation.saturating_add(1))
        .unwrap_or(1);
    config.record_peer(
        profile_name,
        config::PeerRecord {
            node_id,
            node_name,
            service_instance_id: descriptor
                .and_then(|descriptor| descriptor_string_field(descriptor, "service_instance_id"))
                .or(existing.service_instance_id),
            version,
            control_api_version,
            peer_protocol_version,
            features,
            os,
            arch,
            os_user: descriptor
                .and_then(|descriptor| descriptor_string_field(descriptor, "os_user"))
                .or(existing.os_user),
            data_dir: descriptor
                .and_then(|descriptor| descriptor_string_field(descriptor, "data_dir"))
                .map(Into::into)
                .or(existing.data_dir),
            target: Some(result.target.clone()),
            trust: Some("ssh-token-rotate".to_string()),
            remote_path: Some(result.remote_path.clone()),
            control_endpoint: Some(control_endpoint),
            transport: Some(result.remote_tcp),
            tls_transport: result.remote_tls_transport,
            quic_transport: result.remote_quic_transport,
            transport_protocols,
            token: Some(result.remote_token.clone()),
            token_metadata: result.token_metadata.clone().or_else(|| {
                Some(config::TokenMetadata::rotated(
                    "peer-control-transport",
                    token_generation,
                ))
            }),
            tls_server_cert_fingerprint: descriptor
                .and_then(|descriptor| descriptor.pointer("/auth/tls_server_cert_fingerprint"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or(existing.tls_server_cert_fingerprint),
            tls_client_ca_fingerprint: descriptor
                .and_then(|descriptor| descriptor.pointer("/auth/tls_client_ca_fingerprint"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or(existing.tls_client_ca_fingerprint),
            ..Default::default()
        },
    );
}

pub(crate) fn record_remote_descriptor_profile(
    config: &mut config::AppConfig,
    profile_name: &str,
    result: &RemoteDescriptorResult,
) -> Result<()> {
    let profile = config.profiles.entry(profile_name.to_string()).or_default();
    if profile.target.is_none() {
        profile.target = Some(result.target.clone());
    }
    profile.remote_path = Some(result.remote_path.clone());
    profile.remote_control = Some(result.remote_control);
    profile.remote_tcp = Some(result.remote_tcp);
    profile.remote_tls = result.remote_tls_transport;
    profile.remote_quic = result.remote_quic_transport;
    profile.remote_transport = Some("auto".to_string());
    if let Some(token) = &result.remote_token {
        profile.remote_token = Some(token.clone());
    }
    let node_id = result
        .descriptor
        .get("node_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let node_name = result
        .descriptor
        .get("node_name")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let version = descriptor_string_field(&result.descriptor, "version");
    let control_api_version = descriptor_u16_field(&result.descriptor, "control_api_version");
    let peer_protocol_version = descriptor_u16_field(&result.descriptor, "peer_protocol_version");
    let features = descriptor_string_array_field(&result.descriptor, "features");
    let os = descriptor_string_field(&result.descriptor, "os");
    let arch = descriptor_string_field(&result.descriptor, "arch");
    let os_user = descriptor_string_field(&result.descriptor, "os_user");
    let data_dir = descriptor_string_field(&result.descriptor, "data_dir").map(Into::into);
    let service_instance_id = descriptor_string_field(&result.descriptor, "service_instance_id");
    let tls_server_cert_fingerprint = result
        .descriptor
        .pointer("/auth/tls_server_cert_fingerprint")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let tls_client_ca_fingerprint = result
        .descriptor
        .pointer("/auth/tls_client_ca_fingerprint")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let control_endpoint = result
        .descriptor
        .pointer("/endpoints/control")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("tcp://{}", result.remote_control));
    config.record_peer(
        profile_name,
        config::PeerRecord {
            node_id,
            node_name,
            service_instance_id,
            version,
            control_api_version,
            peer_protocol_version,
            features,
            os,
            arch,
            os_user,
            data_dir,
            target: Some(result.target.clone()),
            trust: Some("ssh-refresh".to_string()),
            remote_path: Some(result.remote_path.clone()),
            control_endpoint: Some(control_endpoint),
            transport: Some(result.remote_tcp),
            tls_transport: result.remote_tls_transport,
            quic_transport: result.remote_quic_transport,
            transport_protocols: descriptor_protocols(&result.descriptor)
                .unwrap_or_else(|| remote_descriptor_protocols(result)),
            token: result.remote_token.clone(),
            token_metadata: result
                .descriptor
                .pointer("/auth/token_metadata")
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .or_else(|| {
                    result
                        .remote_token
                        .as_ref()
                        .map(|_| config::TokenMetadata::new("peer-control-transport"))
                }),
            tls_server_cert_fingerprint,
            tls_client_ca_fingerprint,
            ..Default::default()
        },
    );
    config.save_default()
}

pub(crate) fn record_remote_install_profile(
    config: &mut config::AppConfig,
    profile_name: &str,
    result: &RemoteInstallResult,
) -> Result<()> {
    let profile = config.profiles.entry(profile_name.to_string()).or_default();
    if profile.target.is_none() {
        profile.target = Some(result.target.clone());
    }
    profile.remote_path = Some(result.remote_path.clone());
    profile.remote_tcp = Some(result.remote_tcp);
    profile.remote_control = Some(result.remote_control);
    profile.remote_tls = result.remote_tls_transport;
    profile.remote_quic = result.remote_quic_transport;
    profile.remote_transport = Some("auto".to_string());
    if let Some(token) = &result.remote_token {
        profile.remote_token = Some(token.clone());
    }
    let descriptor = result.descriptor.as_ref();
    let node_id = descriptor
        .and_then(|descriptor| descriptor.get("node_id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| result.remote_node_id.clone());
    let node_name = descriptor
        .and_then(|descriptor| descriptor.get("node_name"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| result.remote_node_name.clone());
    let version = descriptor
        .and_then(|descriptor| descriptor_string_field(descriptor, "version"))
        .or_else(|| Some(env!("CARGO_PKG_VERSION").to_string()));
    let control_api_version = descriptor
        .and_then(|descriptor| descriptor_u16_field(descriptor, "control_api_version"))
        .or_else(|| Some(node_daemon::control_api_version()));
    let peer_protocol_version = descriptor
        .and_then(|descriptor| descriptor_u16_field(descriptor, "peer_protocol_version"))
        .or_else(|| Some(node_daemon::peer_protocol_version()));
    let features = descriptor
        .map(|descriptor| descriptor_string_array_field(descriptor, "features"))
        .filter(|features| !features.is_empty())
        .unwrap_or_else(node_daemon::peer_protocol_features);
    let control_endpoint = descriptor
        .and_then(|descriptor| descriptor.pointer("/endpoints/control"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("tcp://{}", result.remote_control));
    let transport_protocols = descriptor
        .and_then(descriptor_protocols)
        .unwrap_or_else(|| remote_transport_protocols(result));
    config.record_peer(
        profile_name,
        config::PeerRecord {
            node_id,
            node_name,
            service_instance_id: descriptor
                .and_then(|descriptor| descriptor_string_field(descriptor, "service_instance_id")),
            version,
            control_api_version,
            peer_protocol_version,
            features,
            os: descriptor.and_then(|descriptor| descriptor_string_field(descriptor, "os")),
            arch: descriptor.and_then(|descriptor| descriptor_string_field(descriptor, "arch")),
            os_user: descriptor
                .and_then(|descriptor| descriptor_string_field(descriptor, "os_user")),
            data_dir: descriptor
                .and_then(|descriptor| descriptor_string_field(descriptor, "data_dir"))
                .map(Into::into),
            target: Some(result.target.clone()),
            trust: Some("ssh-bootstrap".to_string()),
            remote_path: Some(result.remote_path.clone()),
            control_endpoint: Some(control_endpoint),
            transport: Some(result.remote_tcp),
            tls_transport: result.remote_tls_transport,
            quic_transport: result.remote_quic_transport,
            transport_protocols,
            token: result.remote_token.clone(),
            token_metadata: result
                .descriptor
                .as_ref()
                .and_then(|descriptor| descriptor.pointer("/auth/token_metadata"))
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .or_else(|| {
                    result
                        .remote_token
                        .as_ref()
                        .map(|_| config::TokenMetadata::new("peer-control-transport"))
                }),
            tls_server_cert_fingerprint: descriptor
                .and_then(|descriptor| descriptor.pointer("/auth/tls_server_cert_fingerprint"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            tls_client_ca_fingerprint: descriptor
                .and_then(|descriptor| descriptor.pointer("/auth/tls_client_ca_fingerprint"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            ..Default::default()
        },
    );
    config.save_default()
}

fn remote_transport_protocols(result: &RemoteInstallResult) -> Vec<String> {
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
