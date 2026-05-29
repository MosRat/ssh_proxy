use std::net::SocketAddr;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{cli, config};

#[derive(Debug, Clone)]
pub(crate) struct TransportSelection {
    pub(crate) transport: cli::RemoteTransport,
    pub(crate) source: String,
    pub(crate) reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ConnectionDecision {
    pub(crate) selected_transport: String,
    pub(crate) source: String,
    pub(crate) reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) endpoint: Option<String>,
    #[serde(default)]
    pub(crate) requires_external_ssh: bool,
}

impl ConnectionDecision {
    pub(crate) fn from_transport(
        transport: cli::RemoteTransport,
        source: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            selected_transport: remote_transport_name(transport).to_string(),
            source: source.into(),
            reason: reason.into(),
            endpoint: None,
            requires_external_ssh: matches!(transport, cli::RemoteTransport::Exec),
        }
    }

    pub(crate) fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }
}

pub(crate) fn persistent_peer_ready(peer: Option<&config::PeerRecord>) -> bool {
    peer.is_some_and(|peer| {
        peer.remote_path.is_some()
            && peer.control_endpoint.is_some()
            && (peer.transport.is_some()
                || peer.tls_transport.is_some()
                || peer.quic_transport.is_some())
    })
}

pub(crate) fn transport_selection_policy(
    args: &cli::RouteArgs,
    profile: Option<&config::ProxyProfile>,
    defaults: &config::ProxyProfile,
    remote_quic: Option<SocketAddr>,
    remote_tls: Option<SocketAddr>,
    allow_plain_tcp: bool,
    remote_side_listens: bool,
    persistent_peer_ready: bool,
) -> Result<TransportSelection> {
    if args.remote_transport != cli::RemoteTransport::Auto {
        return Ok(TransportSelection {
            transport: args.remote_transport,
            source: "cli".to_string(),
            reason: format!(
                "selected by --remote-transport {}",
                remote_transport_name(args.remote_transport)
            ),
        });
    }

    if let Some(value) = profile.and_then(|profile| profile.remote_transport.as_deref()) {
        let transport = parse_remote_transport(value)?;
        if transport != cli::RemoteTransport::Auto {
            return Ok(TransportSelection {
                transport,
                source: "profile".to_string(),
                reason: "selected by target profile remote_transport".to_string(),
            });
        }
    }

    if let Some(addr) = remote_tls {
        return Ok(TransportSelection {
            transport: cli::RemoteTransport::TlsTcp,
            source: "topology".to_string(),
            reason: format!(
                "direct TLS/TCP peer endpoint {addr} is configured; TLS is the production direct default"
            ),
        });
    }

    if let Some(addr) = remote_quic {
        return Ok(TransportSelection {
            transport: cli::RemoteTransport::Quic,
            source: "topology".to_string(),
            reason: format!(
                "direct QUIC peer endpoint {addr} is configured; framed QUIC is selected while quic-native remains opt-in"
            ),
        });
    }

    if let Some(value) = defaults.remote_transport.as_deref() {
        let transport = parse_remote_transport(value)?;
        match transport {
            cli::RemoteTransport::Auto => {}
            cli::RemoteTransport::PlainTcp if allow_plain_tcp => {
                let source = plain_tcp_auto_source(args, profile, defaults)
                    .unwrap_or("benchmark-tuned default");
                return Ok(TransportSelection {
                    transport,
                    source: source.to_string(),
                    reason: plain_tcp_selection_reason(source),
                });
            }
            cli::RemoteTransport::PlainTcp => {}
            _ => {
                return Ok(TransportSelection {
                    transport,
                    source: "defaults".to_string(),
                    reason: "selected by [defaults].remote_transport".to_string(),
                });
            }
        }
    }

    if allow_plain_tcp {
        let source = plain_tcp_auto_source(args, profile, defaults).unwrap_or("cli");
        return Ok(TransportSelection {
            transport: cli::RemoteTransport::PlainTcp,
            source: source.to_string(),
            reason: plain_tcp_selection_reason(source),
        });
    }

    if persistent_peer_ready {
        return Ok(TransportSelection {
            transport: cli::RemoteTransport::Tcp,
            source: "peer-default".to_string(),
            reason: "persistent remote peer is recorded; using SPX over Rust SSH direct-tcpip to the peer transport".to_string(),
        });
    }

    let workload = if args.tcp_target.is_some() {
        "fixed --tcp-target route"
    } else if remote_side_listens {
        "remote-owned proxy route"
    } else {
        "SOCKS/HTTP proxy route"
    };
    Ok(TransportSelection {
        transport: cli::RemoteTransport::SshNative,
        source: "topology".to_string(),
        reason: format!(
            "no reachable direct peer transport is configured for this {workload}; using ssh-native direct-tcpip as the SSH-only simple egress default"
        ),
    })
}

pub(crate) fn direct_endpoint_for_peer(peer: &config::PeerRecord) -> Option<SocketAddr> {
    peer.tls_transport
        .or(peer.quic_transport)
        .or(peer.transport)
}

pub(crate) fn remote_transport_name(transport: cli::RemoteTransport) -> &'static str {
    match transport {
        cli::RemoteTransport::Auto => "auto",
        cli::RemoteTransport::SshNative => "ssh-native",
        cli::RemoteTransport::QuicNative => "quic-native",
        cli::RemoteTransport::Quic => "quic",
        cli::RemoteTransport::TlsTcp => "tls-tcp",
        cli::RemoteTransport::PlainTcp => "plain-tcp",
        cli::RemoteTransport::Exec => "ssh-exec",
        cli::RemoteTransport::Tcp => "ssh-direct-tcpip",
    }
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

pub(crate) fn parse_remote_transport(value: &str) -> Result<cli::RemoteTransport> {
    match value.to_ascii_lowercase().as_str() {
        "auto" => Ok(cli::RemoteTransport::Auto),
        "quic-native" | "quic_native" | "native-quic" | "native_quic" => {
            Ok(cli::RemoteTransport::QuicNative)
        }
        "ssh-native" | "ssh_native" | "native-ssh" | "native_ssh" => {
            Ok(cli::RemoteTransport::SshNative)
        }
        "quic" => Ok(cli::RemoteTransport::Quic),
        "tls-tcp" | "tls_tcp" | "tls" => Ok(cli::RemoteTransport::TlsTcp),
        "plain-tcp" | "plain_tcp" | "tcp-plain" | "direct-tcp" | "direct_tcp" => {
            Ok(cli::RemoteTransport::PlainTcp)
        }
        "exec" => Ok(cli::RemoteTransport::Exec),
        "tcp" => Ok(cli::RemoteTransport::Tcp),
        other => bail!("invalid remote transport value {other:?}"),
    }
}

fn plain_tcp_auto_source(
    args: &cli::RouteArgs,
    profile: Option<&config::ProxyProfile>,
    defaults: &config::ProxyProfile,
) -> Option<&'static str> {
    if args.allow_plain_tcp {
        Some("cli")
    } else if profile.and_then(|profile| profile.allow_plain_tcp) == Some(true) {
        Some("profile")
    } else if defaults.allow_plain_tcp == Some(true) {
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

    fn route_args() -> cli::RouteArgs {
        cli::RouteArgs {
            target: "peer".to_string(),
            direction: cli::RouteDirection::LocalUsesRemote,
            connect_mode: cli::RouteConnectMode::Auto,
            port: 18080,
            bind: "127.0.0.1".parse().unwrap(),
            tcp_target: None,
            endpoint: "tcp://127.0.0.1:1".to_string(),
            token: None,
            ssh_args: Vec::new(),
            user: None,
            ssh_port: None,
            identity: Vec::new(),
            config: None,
            known_hosts: None,
            accept_new: false,
            insecure_ignore_host_key: false,
            jump: Vec::new(),
            remote_path: None,
            remote_bin: None,
            deploy: cli::DeployMode::Auto,
            remote_os: cli::RemoteOs::Auto,
            remote_transport: cli::RemoteTransport::Auto,
            remote_tcp: None,
            remote_control: None,
            remote_quic: None,
            remote_tls: None,
            remote_ca: None,
            remote_name: "localhost".to_string(),
            remote_token: None,
            egress_proxy: None,
            reconnect_delay_secs: None,
            reconnect_max_delay_secs: None,
            connect_timeout_secs: None,
            quic_max_bidi_streams: None,
            quic_stream_receive_window: None,
            quic_receive_window: None,
            quic_keep_alive_interval_secs: None,
            quic_idle_timeout_secs: None,
            transport_pool_size: None,
            workload_hint: None,
            ssh_session_pool_size: None,
            no_reconnect: false,
            local_peer: None,
            allow_plain_tcp: false,
            id: None,
            volatile: false,
            dry_run: true,
            explain: false,
            json: false,
        }
    }

    #[test]
    fn peer_ready_requires_descriptor_and_transport() {
        assert!(!persistent_peer_ready(None));

        let mut peer = config::PeerRecord {
            remote_path: Some("~/.local/bin/ssh_proxy".to_string()),
            control_endpoint: Some("tcp://127.0.0.1:19081".to_string()),
            ..Default::default()
        };
        assert!(!persistent_peer_ready(Some(&peer)));

        peer.transport = Some("127.0.0.1:19080".parse().unwrap());
        assert!(persistent_peer_ready(Some(&peer)));
    }

    #[test]
    fn external_ssh_is_explicit_compatibility() {
        let decision = ConnectionDecision::from_transport(
            cli::RemoteTransport::Exec,
            "cli",
            "explicit emergency compatibility",
        );

        assert_eq!(decision.selected_transport, "ssh-exec");
        assert!(decision.requires_external_ssh);
    }

    #[test]
    fn transport_metadata_classifies_direct_and_ssh_paths() {
        assert_eq!(
            direct_transport_policy(cli::RemoteTransport::TlsTcp),
            json!("production_direct")
        );
        assert_eq!(
            direct_transport_policy(cli::RemoteTransport::PlainTcp),
            json!("lab_baseline")
        );
        assert_eq!(
            tls_peer_auth_mode(
                cli::RemoteTransport::TlsTcp,
                Some("client.crt"),
                Some("client.key")
            ),
            json!("mutual_tls")
        );
        assert_eq!(
            ssh_mode_name(cli::RemoteTransport::Tcp),
            json!("spx-over-ssh-direct")
        );
        assert_eq!(
            ssh_data_plane_reason(cli::RemoteTransport::SshNative, Some("profile")),
            json!("explicit_user_choice")
        );
        assert_eq!(
            ssh_data_plane_reason(cli::RemoteTransport::Exec, None),
            json!("ssh_exec_compatibility")
        );
    }

    #[test]
    fn transport_selection_keeps_single_precedence_chain() {
        let defaults = config::ProxyProfile::default();
        let mut args = route_args();
        args.remote_transport = cli::RemoteTransport::Exec;

        let selected =
            transport_selection_policy(&args, None, &defaults, None, None, false, false, true)
                .expect("explicit transport");
        assert_eq!(selected.transport, cli::RemoteTransport::Exec);
        assert_eq!(selected.source, "cli");

        let args = route_args();
        let profile = config::ProxyProfile {
            remote_transport: Some("tcp".to_string()),
            ..Default::default()
        };
        let selected = transport_selection_policy(
            &args,
            Some(&profile),
            &defaults,
            Some("192.0.2.8:19083".parse().unwrap()),
            Some("192.0.2.8:19082".parse().unwrap()),
            false,
            false,
            true,
        )
        .expect("profile transport");
        assert_eq!(selected.transport, cli::RemoteTransport::Tcp);
        assert_eq!(selected.source, "profile");

        let selected = transport_selection_policy(
            &args,
            None,
            &defaults,
            Some("192.0.2.8:19083".parse().unwrap()),
            Some("192.0.2.8:19082".parse().unwrap()),
            false,
            false,
            true,
        )
        .expect("tls transport");
        assert_eq!(selected.transport, cli::RemoteTransport::TlsTcp);
        assert_eq!(selected.source, "topology");

        let selected = transport_selection_policy(
            &args,
            None,
            &defaults,
            Some("192.0.2.8:19083".parse().unwrap()),
            None,
            false,
            false,
            true,
        )
        .expect("quic transport");
        assert_eq!(selected.transport, cli::RemoteTransport::Quic);

        let selected =
            transport_selection_policy(&args, None, &defaults, None, None, false, false, true)
                .expect("persistent peer transport");
        assert_eq!(selected.transport, cli::RemoteTransport::Tcp);
        assert_eq!(selected.source, "peer-default");

        let selected =
            transport_selection_policy(&args, None, &defaults, None, None, false, false, false)
                .expect("ssh-only transport");
        assert_eq!(selected.transport, cli::RemoteTransport::SshNative);
        assert_eq!(selected.source, "topology");
    }

    #[test]
    fn plain_tcp_default_needs_explicit_trust_source() {
        let args = route_args();
        let mut defaults = config::ProxyProfile {
            remote_transport: Some("plain-tcp".to_string()),
            ..Default::default()
        };

        let selected =
            transport_selection_policy(&args, None, &defaults, None, None, false, false, false)
                .expect("plain tcp disabled");
        assert_eq!(selected.transport, cli::RemoteTransport::SshNative);

        defaults.allow_plain_tcp = Some(true);
        let selected =
            transport_selection_policy(&args, None, &defaults, None, None, true, false, false)
                .expect("plain tcp enabled");
        assert_eq!(selected.transport, cli::RemoteTransport::PlainTcp);
        assert_eq!(selected.source, "benchmark-tuned default");
    }
}
