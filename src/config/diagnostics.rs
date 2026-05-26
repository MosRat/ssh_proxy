use std::{io::Read, net::SocketAddr};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::{cli, paths, peer_transport};

use super::{AppConfig, CONFIG_SCHEMA_VERSION, PeerRecord, TokenMetadata};

pub(super) fn inspect(config: &AppConfig) -> Value {
    let profiles = sorted_profiles(config)
        .into_iter()
        .map(|(name, profile)| {
            json!({
                "name": name,
                "target": profile.target.as_deref().unwrap_or(name),
                "listen": profile.listen.map(|addr| addr.to_string()),
                "ssh": {
                    "user": profile.user,
                    "port": profile.port,
                    "identity_files": profile.identity,
                    "config": profile.config,
                    "known_hosts": profile.known_hosts,
                    "accept_new": profile.accept_new,
                    "jump": profile.jump,
                },
                "remote": {
                    "path": profile.remote_path,
                    "bin": profile.remote_bin,
                    "os": profile.remote_os,
                    "transport": profile.remote_transport,
                    "tcp": profile.remote_tcp.map(|addr| addr.to_string()),
                    "control": profile.remote_control.map(|addr| addr.to_string()),
                    "quic": profile.remote_quic.map(|addr| addr.to_string()),
                    "tls": profile.remote_tls.map(|addr| addr.to_string()),
                    "allow_plain_tcp": profile.allow_plain_tcp,
                    "token": profile.remote_token.is_some(),
                },
                "certs": {
                    "remote_ca": profile.remote_ca,
                    "remote_name": profile.remote_name,
                    "remote_client_cert": profile.remote_client_cert,
                    "remote_client_key": profile.remote_client_key,
                },
                "reconnect": {
                    "delay_secs": profile.reconnect_delay_secs,
                    "max_delay_secs": profile.reconnect_max_delay_secs,
                    "connect_timeout_secs": profile.connect_timeout_secs,
                    "disabled": profile.no_reconnect,
                }
            })
        })
        .collect::<Vec<_>>();
    let peers = sorted_peers(config)
        .into_iter()
        .map(|(alias, peer)| {
            json!({
                "alias": alias,
                "node_id": peer.node_id,
                "node_name": peer.node_name,
                "version": peer.version,
                "control_api_version": peer.control_api_version,
                "peer_protocol_version": peer.peer_protocol_version,
                "features": peer.features,
                "os": peer.os,
                "arch": peer.arch,
                "target": peer.target,
                "trust": peer.trust,
                "remote_path": peer.remote_path,
                "control_endpoint": peer.control_endpoint,
                "transport": peer.transport.map(|addr| addr.to_string()),
                "tls_transport": peer.tls_transport.map(|addr| addr.to_string()),
                "quic_transport": peer.quic_transport.map(|addr| addr.to_string()),
                "transport_protocols": peer.known_transport_protocols(),
                "auth": {
                    "token": peer.token.is_some(),
                    "token_metadata": peer.token_metadata,
                },
                "last_seen_unix": peer.last_seen_unix,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "ok": true,
        "kind": "config_inspect",
        "schema_version": config.schema_version,
        "current_schema_version": CONFIG_SCHEMA_VERSION,
        "identity": {
            "node_id": config.identity.node_id,
            "node_name": config.identity.node_name,
            "secret": config.identity.secret.is_some(),
            "cert": config.identity.cert,
            "key": config.identity.key,
            "ca": config.identity.ca,
        },
        "daemon": {
            "control_listen": config.daemon.control_listen.map(|addr| addr.to_string()),
            "control_endpoint": config.daemon.control_endpoint,
            "transport_listen": config.daemon.transport_listen.map(|addr| addr.to_string()),
            "tls_transport_listen": config.daemon.tls_transport_listen.map(|addr| addr.to_string()),
            "quic_transport_listen": config.daemon.quic_transport_listen.map(|addr| addr.to_string()),
            "routes_path": config.daemon.routes_path,
            "route_autostart": config.daemon.route_autostart,
            "report_to": config.daemon.report_to,
            "auth": {
                "token": config.daemon.token.is_some(),
                "token_metadata": config.daemon.token_metadata,
            },
            "certs": {
                "tls_cert": config.daemon.tls_cert,
                "tls_key": config.daemon.tls_key,
                "tls_client_ca": config.daemon.tls_client_ca,
            }
        },
        "counts": {
            "profiles": config.profiles.len(),
            "peers": config.peers.len(),
        },
        "profiles": profiles,
        "peers": peers,
    })
}

pub(super) fn export_descriptor(config: &AppConfig) -> Value {
    let has_token = config.daemon.token.is_some();
    let os_user = whoami::username().unwrap_or_else(|_| "unknown-user".to_string());
    let control_endpoint = config.daemon.control_endpoint.clone().or_else(|| {
        config
            .daemon
            .control_listen
            .map(|addr| format!("tcp://{addr}"))
    });
    let service_instance_id = format!(
        "{}@{}:{}",
        config
            .identity
            .node_id
            .as_deref()
            .unwrap_or("uninitialized-node"),
        os_user,
        control_endpoint.as_deref().unwrap_or("control-unset")
    );
    let token_metadata = config.daemon.token_metadata.clone();
    let token_generation = token_metadata.as_ref().map(|metadata| metadata.generation);
    let tls_server_cert_fingerprint = config
        .daemon
        .tls_cert
        .as_ref()
        .and_then(|path| super::file_sha256_fingerprint(&super::expand_path(path)));
    let tls_client_ca_fingerprint = config
        .daemon
        .tls_client_ca
        .as_ref()
        .and_then(|path| super::file_sha256_fingerprint(&super::expand_path(path)));
    json!({
        "ok": true,
        "kind": "peer_descriptor",
        "source": "config-export",
        "schema_version": config.schema_version,
        "node_id": config.identity.node_id,
        "node_name": config.identity.node_name,
        "service_instance_id": service_instance_id,
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "os_user": os_user,
        "data_dir": paths::app_home().ok(),
        "control_api_version": 1,
        "peer_protocol_version": peer_transport::PEER_VERSION,
        "features": peer_transport::default_features(),
        "feature_bits": peer_transport::default_feature_bits(),
        "endpoints": {
            "control": control_endpoint,
            "transport": config.daemon.transport_listen.map(|addr| addr.to_string()),
            "tls_transport": config.daemon.tls_transport_listen.map(|addr| addr.to_string()),
            "quic_transport": config.daemon.quic_transport_listen.map(|addr| addr.to_string()),
        },
        "transport_protocols": descriptor_transport_protocols(config),
        "auth": {
            "control_token": has_token,
            "transport_token": has_token,
            "token_metadata": token_metadata,
            "token_generation": token_generation,
            "tls_server_cert": config.daemon.tls_cert.is_some() && config.daemon.tls_key.is_some(),
            "tls_client_ca": config.daemon.tls_client_ca.is_some(),
            "tls_server_cert_fingerprint": tls_server_cert_fingerprint,
            "tls_client_ca_fingerprint": tls_client_ca_fingerprint,
        },
        "routes_path": config.daemon.routes_path,
        "route_autostart": config.daemon.route_autostart,
    })
}

pub(super) fn import_peer_descriptor(
    config: &mut AppConfig,
    args: &cli::ConfigImportDescriptorArgs,
) -> Result<()> {
    let descriptor = read_descriptor_json(&args.path)?;
    if descriptor.get("ok").and_then(Value::as_bool) == Some(false) {
        bail!("descriptor reports ok=false");
    }
    let target = args
        .target
        .clone()
        .or_else(|| {
            descriptor
                .get("target")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| args.alias.clone());
    let control_endpoint = descriptor
        .pointer("/endpoints/control")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            descriptor
                .get("control_endpoint")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        });
    let transport = descriptor
        .pointer("/endpoints/transport")
        .and_then(Value::as_str)
        .and_then(parse_socket_or_tcp_endpoint);
    let tls_transport = descriptor
        .pointer("/endpoints/tls_transport")
        .and_then(Value::as_str)
        .and_then(parse_socket_or_tcp_endpoint);
    let quic_transport = descriptor
        .pointer("/endpoints/quic_transport")
        .and_then(Value::as_str)
        .and_then(parse_socket_or_tcp_endpoint);
    let protocols = descriptor
        .get("transport_protocols")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            let mut protocols = Vec::new();
            if quic_transport.is_some() {
                protocols.push("quic".to_string());
            }
            if tls_transport.is_some() {
                protocols.push("tls-tcp".to_string());
            }
            if transport.is_some() {
                protocols.push("plain-tcp".to_string());
            }
            protocols
        });
    let token_metadata = descriptor
        .pointer("/auth/token_metadata")
        .and_then(|value| serde_json::from_value(value.clone()).ok());
    let version = descriptor
        .get("version")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let control_api_version = descriptor
        .get("control_api_version")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok());
    let peer_protocol_version = descriptor
        .get("peer_protocol_version")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok());
    let features = descriptor
        .get("features")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let os = descriptor
        .get("os")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let arch = descriptor
        .get("arch")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let os_user = descriptor
        .get("os_user")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let data_dir = descriptor
        .get("data_dir")
        .and_then(Value::as_str)
        .map(Into::into);
    let service_instance_id = descriptor
        .get("service_instance_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let tls_server_cert_fingerprint = descriptor
        .pointer("/auth/tls_server_cert_fingerprint")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let tls_client_ca_fingerprint = descriptor
        .pointer("/auth/tls_client_ca_fingerprint")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    let profile = config.profiles.entry(args.alias.clone()).or_default();
    if profile.target.is_none() {
        profile.target = Some(target.clone());
    }
    profile.remote_control = control_endpoint
        .as_deref()
        .and_then(parse_socket_or_tcp_endpoint)
        .or(profile.remote_control);
    profile.remote_tcp = transport.or(profile.remote_tcp);
    profile.remote_tls = tls_transport.or(profile.remote_tls);
    profile.remote_quic = quic_transport.or(profile.remote_quic);
    profile.remote_transport = Some("auto".to_string());
    if let Some(token) = &args.token {
        profile.remote_token = Some(token.clone());
    }

    config.record_peer(
        &args.alias,
        PeerRecord {
            node_id: descriptor
                .get("node_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            node_name: descriptor
                .get("node_name")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            service_instance_id,
            version,
            control_api_version,
            peer_protocol_version,
            features,
            os,
            arch,
            os_user,
            data_dir,
            target: Some(target),
            trust: Some(args.trust.clone()),
            control_endpoint,
            transport,
            tls_transport,
            quic_transport,
            transport_protocols: protocols,
            token: args.token.clone(),
            token_metadata: token_metadata.or_else(|| {
                args.token
                    .as_ref()
                    .map(|_| TokenMetadata::new("peer-control-transport"))
            }),
            tls_server_cert_fingerprint,
            tls_client_ca_fingerprint,
            ..Default::default()
        },
    );
    Ok(())
}

fn sorted_profiles(config: &AppConfig) -> Vec<(&String, &super::ProxyProfile)> {
    let mut profiles = config.profiles.iter().collect::<Vec<_>>();
    profiles.sort_by(|(left, _), (right, _)| left.cmp(right));
    profiles
}

fn sorted_peers(config: &AppConfig) -> Vec<(&String, &PeerRecord)> {
    let mut peers = config.peers.iter().collect::<Vec<_>>();
    peers.sort_by(|(left, _), (right, _)| left.cmp(right));
    peers
}

fn descriptor_transport_protocols(config: &AppConfig) -> Vec<String> {
    let mut protocols = Vec::new();
    if config.daemon.quic_transport_listen.is_some() {
        protocols.push("quic".to_string());
    }
    if config.daemon.tls_transport_listen.is_some() {
        protocols.push("tls-tcp".to_string());
    }
    if config.daemon.transport_listen.is_some() {
        protocols.push("plain-tcp".to_string());
    }
    protocols
}

fn read_descriptor_json(path: &str) -> Result<Value> {
    let text = if path == "-" {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("failed to read descriptor JSON from stdin")?;
        text
    } else {
        std::fs::read_to_string(path)
            .with_context(|| format!("failed to read descriptor JSON from {path}"))?
    };
    serde_json::from_str(&text).context("failed to parse descriptor JSON")
}

fn parse_socket_or_tcp_endpoint(value: &str) -> Option<SocketAddr> {
    value.strip_prefix("tcp://").unwrap_or(value).parse().ok()
}
