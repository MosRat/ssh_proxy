use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::{cli, peer_lifecycle};

pub(crate) fn remote_transport_name(transport: cli::RemoteTransport) -> &'static str {
    peer_lifecycle::connection::remote_transport_name(transport)
}

pub(crate) fn direct_transport_policy(transport: cli::RemoteTransport) -> Value {
    match transport {
        cli::RemoteTransport::TlsTcp => json!("production_direct"),
        cli::RemoteTransport::PlainTcp => json!("lab_baseline"),
        cli::RemoteTransport::Quic | cli::RemoteTransport::QuicNative => json!("experimental"),
        _ => Value::Null,
    }
}

pub(crate) fn direct_transport_policy_reason(transport: cli::RemoteTransport) -> Value {
    match transport {
        cli::RemoteTransport::TlsTcp => json!(
            "TLS/TCP SPX is the production direct baseline because it keeps the stable SPX data plane while adding peer encryption and certificate identity"
        ),
        cli::RemoteTransport::PlainTcp => json!(
            "Plain TCP SPX is a lab or explicitly trusted baseline only; it is not selected as the production default because the data path is not encrypted"
        ),
        cli::RemoteTransport::Quic | cli::RemoteTransport::QuicNative => json!(
            "QUIC direct transport remains experimental until throughput and recovery behavior close the gap with TLS/TCP SPX"
        ),
        _ => Value::Null,
    }
}

pub(crate) fn tls_peer_auth_mode<T, U>(
    transport: cli::RemoteTransport,
    client_cert: Option<T>,
    client_key: Option<U>,
) -> Value {
    if !matches!(transport, cli::RemoteTransport::TlsTcp) {
        return Value::Null;
    }
    match (client_cert.is_some(), client_key.is_some()) {
        (true, true) => json!("mutual_tls"),
        (false, false) => json!("server_auth"),
        _ => json!("invalid_client_auth_config"),
    }
}

pub(crate) fn ssh_mode_name(transport: cli::RemoteTransport) -> Value {
    match transport {
        cli::RemoteTransport::SshNative => json!("native-direct-tcpip"),
        cli::RemoteTransport::Tcp => json!("spx-over-ssh-direct"),
        cli::RemoteTransport::Exec => json!("ssh-exec-helper"),
        _ => Value::Null,
    }
}

pub(crate) fn ssh_mode_reason(transport: cli::RemoteTransport) -> Value {
    match transport {
        cli::RemoteTransport::SshNative => json!(
            "ssh-native opens russh direct-tcpip channels to each requested target; use it for simple SSH-only local egress because it avoids remote daemon and SPX framed data-plane overhead"
        ),
        cli::RemoteTransport::Tcp => json!(
            "spx-over-ssh-direct opens SSH direct-tcpip to the remote daemon transport and keeps SPX daemon semantics; use it when remote daemon policy, token auth, route restore, or SPX UDP behavior is required"
        ),
        cli::RemoteTransport::Exec => json!(
            "ssh-exec-helper starts a temporary remote helper over SSH; keep it as a compatibility path when no persistent remote daemon transport is available"
        ),
        _ => Value::Null,
    }
}

pub(crate) fn ssh_data_plane_reason(
    transport: cli::RemoteTransport,
    selection_source: Option<&str>,
) -> Value {
    if matches!(selection_source, Some("cli" | "profile")) {
        return match transport {
            cli::RemoteTransport::SshNative
            | cli::RemoteTransport::Tcp
            | cli::RemoteTransport::Exec => json!("explicit_user_choice"),
            _ => Value::Null,
        };
    }
    match transport {
        cli::RemoteTransport::SshNative => json!("simple_egress"),
        cli::RemoteTransport::Tcp => json!("daemon_policy_required"),
        cli::RemoteTransport::Exec => json!("ssh_exec_compatibility"),
        _ => Value::Null,
    }
}

pub(crate) fn parse_remote_os(value: &str) -> Result<cli::RemoteOs> {
    match value.to_ascii_lowercase().as_str() {
        "auto" => Ok(cli::RemoteOs::Auto),
        "unix" | "linux" | "macos" => Ok(cli::RemoteOs::Unix),
        "windows" => Ok(cli::RemoteOs::Windows),
        other => bail!("invalid remote_os value {other:?}"),
    }
}

#[cfg(test)]
pub(crate) fn parse_remote_transport(value: &str) -> Result<cli::RemoteTransport> {
    peer_lifecycle::connection::parse_remote_transport(value)
}
