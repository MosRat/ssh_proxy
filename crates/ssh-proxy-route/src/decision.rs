use std::net::SocketAddr;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ssh_proxy_core::model::TransportMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportSelection {
    pub transport: TransportMode,
    pub source: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionDecision {
    pub selected_transport: String,
    pub source: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub requires_external_ssh: bool,
}

impl ConnectionDecision {
    pub fn from_transport(
        transport: TransportMode,
        source: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            selected_transport: remote_transport_name(transport).to_string(),
            source: source.into(),
            reason: reason.into(),
            endpoint: None,
            requires_external_ssh: matches!(transport, TransportMode::Exec),
        }
    }

    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PeerReadinessInput {
    pub has_remote_path: bool,
    pub has_control_endpoint: bool,
    pub has_transport: bool,
    pub has_tls_transport: bool,
    pub has_quic_transport: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransportSelectionPolicyInput<'a> {
    pub requested_transport: TransportMode,
    pub profile_remote_transport: Option<&'a str>,
    pub defaults_remote_transport: Option<&'a str>,
    pub remote_quic: Option<SocketAddr>,
    pub remote_tls: Option<SocketAddr>,
    pub allow_plain_tcp: bool,
    pub cli_allow_plain_tcp: bool,
    pub profile_allow_plain_tcp: Option<bool>,
    pub defaults_allow_plain_tcp: Option<bool>,
    pub tcp_target_present: bool,
    pub remote_side_listens: bool,
    pub persistent_peer_ready: bool,
}

pub fn persistent_peer_ready(input: PeerReadinessInput) -> bool {
    input.has_remote_path
        && input.has_control_endpoint
        && (input.has_transport || input.has_tls_transport || input.has_quic_transport)
}

pub fn direct_endpoint_for_peer(
    transport: Option<SocketAddr>,
    tls_transport: Option<SocketAddr>,
    quic_transport: Option<SocketAddr>,
) -> Option<SocketAddr> {
    tls_transport.or(quic_transport).or(transport)
}

pub fn transport_selection_policy(
    input: &TransportSelectionPolicyInput<'_>,
) -> Result<TransportSelection, String> {
    if input.requested_transport != TransportMode::Auto {
        return Ok(TransportSelection {
            transport: input.requested_transport,
            source: "cli".to_string(),
            reason: format!(
                "selected by --remote-transport {}",
                remote_transport_name(input.requested_transport)
            ),
        });
    }

    if let Some(value) = input.profile_remote_transport {
        let transport = parse_transport_mode(value)?;
        if transport != TransportMode::Auto {
            return Ok(TransportSelection {
                transport,
                source: "profile".to_string(),
                reason: "selected by target profile remote_transport".to_string(),
            });
        }
    }

    if let Some(addr) = input.remote_tls {
        return Ok(TransportSelection {
            transport: TransportMode::TlsTcp,
            source: "topology".to_string(),
            reason: format!(
                "direct TLS/TCP peer endpoint {addr} is configured; TLS is the production direct default"
            ),
        });
    }

    if let Some(addr) = input.remote_quic {
        return Ok(TransportSelection {
            transport: TransportMode::Quic,
            source: "topology".to_string(),
            reason: format!(
                "direct QUIC peer endpoint {addr} is configured; framed QUIC is selected while quic-native remains opt-in"
            ),
        });
    }

    if let Some(value) = input.defaults_remote_transport {
        let transport = parse_transport_mode(value)?;
        match transport {
            TransportMode::Auto => {}
            TransportMode::PlainTcp if input.allow_plain_tcp => {
                let source = plain_tcp_auto_source(input).unwrap_or("benchmark-tuned default");
                return Ok(TransportSelection {
                    transport,
                    source: source.to_string(),
                    reason: plain_tcp_selection_reason(source),
                });
            }
            TransportMode::PlainTcp => {}
            _ => {
                return Ok(TransportSelection {
                    transport,
                    source: "defaults".to_string(),
                    reason: "selected by [defaults].remote_transport".to_string(),
                });
            }
        }
    }

    if input.allow_plain_tcp {
        let source = plain_tcp_auto_source(input).unwrap_or("cli");
        return Ok(TransportSelection {
            transport: TransportMode::PlainTcp,
            source: source.to_string(),
            reason: plain_tcp_selection_reason(source),
        });
    }

    if input.persistent_peer_ready {
        return Ok(TransportSelection {
            transport: TransportMode::Tcp,
            source: "peer-default".to_string(),
            reason: "persistent remote peer is recorded; using SPX over Rust SSH direct-tcpip to the peer transport".to_string(),
        });
    }

    let workload = if input.tcp_target_present {
        "fixed --tcp-target route"
    } else if input.remote_side_listens {
        "remote-owned proxy route"
    } else {
        "SOCKS/HTTP proxy route"
    };
    Ok(TransportSelection {
        transport: TransportMode::SshNative,
        source: "topology".to_string(),
        reason: format!(
            "no reachable direct peer transport is configured for this {workload}; using ssh-native direct-tcpip as the SSH-only simple egress default"
        ),
    })
}

pub fn remote_transport_name(transport: TransportMode) -> &'static str {
    match transport {
        TransportMode::Auto => "auto",
        TransportMode::SshNative => "ssh-native",
        TransportMode::QuicNative => "quic-native",
        TransportMode::Quic => "quic",
        TransportMode::TlsTcp => "tls-tcp",
        TransportMode::PlainTcp => "plain-tcp",
        TransportMode::Exec => "ssh-exec",
        TransportMode::Tcp => "ssh-direct-tcpip",
    }
}

pub fn direct_transport_policy(transport: TransportMode) -> Value {
    match transport {
        TransportMode::TlsTcp => json!("production_direct"),
        TransportMode::PlainTcp => json!("lab_baseline"),
        TransportMode::Quic | TransportMode::QuicNative => json!("experimental"),
        _ => Value::Null,
    }
}

pub fn direct_transport_policy_reason(transport: TransportMode) -> Value {
    match transport {
        TransportMode::TlsTcp => json!(
            "TLS/TCP SPX is the production direct baseline because it keeps the stable SPX data plane while adding peer encryption and certificate identity"
        ),
        TransportMode::PlainTcp => json!(
            "Plain TCP SPX is a lab or explicitly trusted baseline only; it is not selected as the production default because the data path is not encrypted"
        ),
        TransportMode::Quic | TransportMode::QuicNative => json!(
            "QUIC direct transport remains experimental until throughput and recovery behavior close the gap with TLS/TCP SPX"
        ),
        _ => Value::Null,
    }
}

pub fn tls_peer_auth_mode(
    transport: TransportMode,
    has_client_cert: bool,
    has_client_key: bool,
) -> Value {
    if !matches!(transport, TransportMode::TlsTcp) {
        return Value::Null;
    }
    match (has_client_cert, has_client_key) {
        (true, true) => json!("mutual_tls"),
        (false, false) => json!("server_auth"),
        _ => json!("invalid_client_auth_config"),
    }
}

pub fn ssh_mode_name(transport: TransportMode) -> Value {
    match transport {
        TransportMode::SshNative => json!("native-direct-tcpip"),
        TransportMode::Tcp => json!("spx-over-ssh-direct"),
        TransportMode::Exec => json!("ssh-exec-helper"),
        _ => Value::Null,
    }
}

pub fn ssh_mode_reason(transport: TransportMode) -> Value {
    match transport {
        TransportMode::SshNative => json!(
            "ssh-native opens russh direct-tcpip channels to each requested target; use it for simple SSH-only local egress because it avoids remote daemon and SPX framed data-plane overhead"
        ),
        TransportMode::Tcp => json!(
            "spx-over-ssh-direct opens SSH direct-tcpip to the remote daemon transport and keeps SPX daemon semantics; use it when remote daemon policy, token auth, route restore, or SPX UDP behavior is required"
        ),
        TransportMode::Exec => json!(
            "ssh-exec-helper starts a temporary remote helper over SSH; keep it as a compatibility path when no persistent remote daemon transport is available"
        ),
        _ => Value::Null,
    }
}

pub fn ssh_data_plane_reason(transport: TransportMode, selection_source: Option<&str>) -> Value {
    if matches!(selection_source, Some("cli" | "profile")) {
        return match transport {
            TransportMode::SshNative | TransportMode::Tcp | TransportMode::Exec => {
                json!("explicit_user_choice")
            }
            _ => Value::Null,
        };
    }
    match transport {
        TransportMode::SshNative => json!("simple_egress"),
        TransportMode::Tcp => json!("daemon_policy_required"),
        TransportMode::Exec => json!("ssh_exec_compatibility"),
        _ => Value::Null,
    }
}

pub fn parse_transport_mode(value: &str) -> Result<TransportMode, String> {
    match value.to_ascii_lowercase().as_str() {
        "auto" => Ok(TransportMode::Auto),
        "quic-native" | "quic_native" | "native-quic" | "native_quic" => {
            Ok(TransportMode::QuicNative)
        }
        "ssh-native" | "ssh_native" | "native-ssh" | "native_ssh" => Ok(TransportMode::SshNative),
        "quic" => Ok(TransportMode::Quic),
        "tls-tcp" | "tls_tcp" | "tls" => Ok(TransportMode::TlsTcp),
        "plain-tcp" | "plain_tcp" | "tcp-plain" | "direct-tcp" | "direct_tcp" => {
            Ok(TransportMode::PlainTcp)
        }
        "exec" => Ok(TransportMode::Exec),
        "tcp" => Ok(TransportMode::Tcp),
        other => Err(format!("invalid remote transport value {other:?}")),
    }
}

fn plain_tcp_auto_source(input: &TransportSelectionPolicyInput<'_>) -> Option<&'static str> {
    if input.cli_allow_plain_tcp {
        Some("cli")
    } else if input.profile_allow_plain_tcp == Some(true) {
        Some("profile")
    } else if input.defaults_allow_plain_tcp == Some(true) {
        Some("benchmark-tuned default")
    } else {
        None
    }
}

fn plain_tcp_selection_reason(source: &str) -> String {
    format!(
        "plain TCP peer transport is enabled by {source}; use only for lab or private trusted links"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> TransportSelectionPolicyInput<'static> {
        TransportSelectionPolicyInput::default()
    }

    #[test]
    fn peer_ready_requires_descriptor_and_transport() {
        assert!(!persistent_peer_ready(PeerReadinessInput::default()));
        assert!(!persistent_peer_ready(PeerReadinessInput {
            has_remote_path: true,
            has_control_endpoint: true,
            ..PeerReadinessInput::default()
        }));
        assert!(persistent_peer_ready(PeerReadinessInput {
            has_remote_path: true,
            has_control_endpoint: true,
            has_transport: true,
            ..PeerReadinessInput::default()
        }));
    }

    #[test]
    fn external_ssh_is_explicit_compatibility() {
        let decision = ConnectionDecision::from_transport(
            TransportMode::Exec,
            "cli",
            "explicit emergency compatibility",
        );

        assert_eq!(decision.selected_transport, "ssh-exec");
        assert!(decision.requires_external_ssh);
    }

    #[test]
    fn transport_metadata_classifies_direct_and_ssh_paths() {
        assert_eq!(
            direct_transport_policy(TransportMode::TlsTcp),
            json!("production_direct")
        );
        assert_eq!(
            direct_transport_policy(TransportMode::PlainTcp),
            json!("lab_baseline")
        );
        assert_eq!(
            tls_peer_auth_mode(TransportMode::TlsTcp, true, true),
            json!("mutual_tls")
        );
        assert_eq!(
            ssh_mode_name(TransportMode::Tcp),
            json!("spx-over-ssh-direct")
        );
        assert_eq!(
            ssh_data_plane_reason(TransportMode::SshNative, Some("profile")),
            json!("explicit_user_choice")
        );
        assert_eq!(
            ssh_data_plane_reason(TransportMode::Exec, None),
            json!("ssh_exec_compatibility")
        );
    }

    #[test]
    fn transport_selection_keeps_single_precedence_chain() {
        let selected = transport_selection_policy(&TransportSelectionPolicyInput {
            requested_transport: TransportMode::Exec,
            persistent_peer_ready: true,
            ..input()
        })
        .expect("explicit transport");
        assert_eq!(selected.transport, TransportMode::Exec);
        assert_eq!(selected.source, "cli");

        let selected = transport_selection_policy(&TransportSelectionPolicyInput {
            profile_remote_transport: Some("tcp"),
            remote_quic: Some("192.0.2.8:19083".parse().unwrap()),
            remote_tls: Some("192.0.2.8:19082".parse().unwrap()),
            persistent_peer_ready: true,
            ..input()
        })
        .expect("profile transport");
        assert_eq!(selected.transport, TransportMode::Tcp);
        assert_eq!(selected.source, "profile");

        let selected = transport_selection_policy(&TransportSelectionPolicyInput {
            remote_quic: Some("192.0.2.8:19083".parse().unwrap()),
            remote_tls: Some("192.0.2.8:19082".parse().unwrap()),
            persistent_peer_ready: true,
            ..input()
        })
        .expect("tls transport");
        assert_eq!(selected.transport, TransportMode::TlsTcp);
        assert_eq!(selected.source, "topology");

        let selected = transport_selection_policy(&TransportSelectionPolicyInput {
            remote_quic: Some("192.0.2.8:19083".parse().unwrap()),
            persistent_peer_ready: true,
            ..input()
        })
        .expect("quic transport");
        assert_eq!(selected.transport, TransportMode::Quic);

        let selected = transport_selection_policy(&TransportSelectionPolicyInput {
            persistent_peer_ready: true,
            ..input()
        })
        .expect("persistent peer transport");
        assert_eq!(selected.transport, TransportMode::Tcp);
        assert_eq!(selected.source, "peer-default");

        let selected = transport_selection_policy(&input()).expect("ssh-only transport");
        assert_eq!(selected.transport, TransportMode::SshNative);
        assert_eq!(selected.source, "topology");
    }

    #[test]
    fn explicit_quic_native_stays_opt_in() {
        let selected = transport_selection_policy(&TransportSelectionPolicyInput {
            requested_transport: TransportMode::QuicNative,
            remote_quic: Some("192.0.2.8:19083".parse().unwrap()),
            ..input()
        })
        .expect("explicit quic-native");

        assert_eq!(selected.transport, TransportMode::QuicNative);
        assert_eq!(selected.source, "cli");
        assert_eq!(remote_transport_name(selected.transport), "quic-native");
        assert_eq!(ssh_mode_name(selected.transport), Value::Null);
        assert_eq!(ssh_mode_reason(selected.transport), Value::Null);
    }

    #[test]
    fn tls_endpoint_beats_unsafe_plain_default() {
        let selected = transport_selection_policy(&TransportSelectionPolicyInput {
            defaults_remote_transport: Some("plain-tcp"),
            remote_tls: Some("192.0.2.8:19082".parse().unwrap()),
            allow_plain_tcp: true,
            defaults_allow_plain_tcp: Some(true),
            ..input()
        })
        .expect("tls topology");

        assert_eq!(selected.transport, TransportMode::TlsTcp);
        assert_eq!(selected.source, "topology");
        assert_eq!(
            direct_transport_policy(selected.transport),
            json!("production_direct")
        );
        assert_eq!(
            tls_peer_auth_mode(selected.transport, false, false),
            json!("server_auth")
        );
    }

    #[test]
    fn plain_tcp_default_needs_explicit_trust_source() {
        let selected = transport_selection_policy(&TransportSelectionPolicyInput {
            defaults_remote_transport: Some("plain-tcp"),
            ..input()
        })
        .expect("plain tcp disabled");
        assert_eq!(selected.transport, TransportMode::SshNative);

        let selected = transport_selection_policy(&TransportSelectionPolicyInput {
            defaults_remote_transport: Some("plain-tcp"),
            allow_plain_tcp: true,
            defaults_allow_plain_tcp: Some(true),
            ..input()
        })
        .expect("plain tcp enabled");
        assert_eq!(selected.transport, TransportMode::PlainTcp);
        assert_eq!(selected.source, "benchmark-tuned default");
    }
}
