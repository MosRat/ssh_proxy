use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use crate::{cli, config};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
