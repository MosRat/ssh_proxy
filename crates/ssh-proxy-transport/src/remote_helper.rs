use std::{fmt, net::SocketAddr, path::PathBuf};

use serde::Serialize;
use ssh_proxy_core::model::{RemotePlatform, TransportMode};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::peer_transport::{PeerProtocol, QuicTransportOptions};

#[path = "remote_helper/client.rs"]
pub mod client;

pub mod intent {
    pub use super::RemoteHelperOpenIntent;
}

pub mod report {
    pub use super::{
        AutoTransportError, RemoteHelperOpenReport, RemoteHelperTimings, TransportCandidateFailure,
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteHelperOpenIntent {
    pub transport: TransportMode,
    pub remote_platform: RemotePlatform,
    pub remote_tcp: SocketAddr,
    pub remote_quic: Option<SocketAddr>,
    pub remote_tls: Option<SocketAddr>,
    pub remote_name: String,
    pub remote_ca: Option<PathBuf>,
    pub remote_client_cert: Option<PathBuf>,
    pub remote_client_key: Option<PathBuf>,
    pub remote_token: Option<String>,
    pub allow_plain_tcp: bool,
    pub connect_timeout_secs: u64,
    pub quic: QuicTransportOptions,
}

impl RemoteHelperOpenIntent {
    pub fn network_hints(&self) -> crate::peer_transport::NetworkHints {
        crate::peer_transport::NetworkHints {
            peer_addr: self
                .remote_quic
                .or(self.remote_tls)
                .or(Some(self.remote_tcp)),
            ssh_available: true,
            allow_plain_tcp: self.allow_plain_tcp,
            prefer_low_latency: true,
        }
    }

    pub fn candidate_protocols(&self) -> Vec<PeerProtocol> {
        match self.transport {
            TransportMode::Auto => {
                crate::peer_transport::implemented_auto_candidates(&self.network_hints())
                    .into_iter()
                    .map(|candidate| candidate.protocol)
                    .collect()
            }
            TransportMode::SshNative => vec![PeerProtocol::SshNative],
            TransportMode::QuicNative => vec![PeerProtocol::QuicNative],
            TransportMode::Quic => vec![PeerProtocol::Quic],
            TransportMode::TlsTcp => vec![PeerProtocol::TlsTcp],
            TransportMode::PlainTcp => vec![PeerProtocol::Tcp],
            TransportMode::Exec => vec![PeerProtocol::SshExec],
            TransportMode::Tcp => vec![PeerProtocol::SshDirect],
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteHelperOpenReport {
    pub selected_protocol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_tcp: Option<SocketAddr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_quic: Option<SocketAddr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_tls: Option<SocketAddr>,
    pub timings: RemoteHelperTimings,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_failures: Vec<TransportCandidateFailure>,
}

impl RemoteHelperOpenReport {
    pub fn from_opened(intent: &RemoteHelperOpenIntent, opened: &OpenedRemoteHelper) -> Self {
        Self {
            selected_protocol: opened.protocol.to_string(),
            remote_tcp: Some(intent.remote_tcp),
            remote_quic: intent.remote_quic,
            remote_tls: intent.remote_tls,
            timings: opened.timings,
            candidate_failures: Vec::new(),
        }
    }
}

pub trait RemoteStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> RemoteStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

pub type BoxedRemoteStream = Box<dyn RemoteStream>;

pub struct OpenedRemoteHelper {
    pub stream: BoxedRemoteStream,
    pub protocol: PeerProtocol,
    pub timings: RemoteHelperTimings,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct RemoteHelperTimings {
    pub ssh_direct_channel_open_latency_ms: Option<u64>,
    pub spx_peer_handshake_latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransportCandidateFailure {
    pub protocol: String,
    pub error: String,
}

#[derive(Debug, Clone)]
pub struct AutoTransportError {
    pub failures: Vec<TransportCandidateFailure>,
}

impl fmt::Display for AutoTransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.failures.is_empty() {
            return f.write_str("no remote transport candidates were usable");
        }
        write!(f, "all remote transport candidates failed: ")?;
        for (index, failure) in self.failures.iter().enumerate() {
            if index > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{}: {}", failure.protocol, failure.error)?;
        }
        Ok(())
    }
}

impl std::error::Error for AutoTransportError {}

pub fn opened_remote<S>(stream: S, protocol: PeerProtocol) -> OpenedRemoteHelper
where
    S: RemoteStream + 'static,
{
    OpenedRemoteHelper {
        stream: Box::new(stream),
        protocol,
        timings: RemoteHelperTimings::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(transport: TransportMode) -> RemoteHelperOpenIntent {
        RemoteHelperOpenIntent {
            transport,
            remote_platform: RemotePlatform::Unix,
            remote_tcp: "127.0.0.1:19080".parse().unwrap(),
            remote_quic: Some("127.0.0.1:19082".parse().unwrap()),
            remote_tls: Some("127.0.0.1:19083".parse().unwrap()),
            remote_name: "localhost".to_string(),
            remote_ca: None,
            remote_client_cert: None,
            remote_client_key: None,
            remote_token: None,
            allow_plain_tcp: false,
            connect_timeout_secs: 30,
            quic: QuicTransportOptions::default(),
        }
    }

    #[test]
    fn intent_maps_explicit_transport_to_protocol_candidate() {
        assert_eq!(
            intent(TransportMode::Tcp).candidate_protocols(),
            vec![PeerProtocol::SshDirect]
        );
        assert_eq!(
            intent(TransportMode::Exec).candidate_protocols(),
            vec![PeerProtocol::SshExec]
        );
        assert_eq!(
            intent(TransportMode::PlainTcp).candidate_protocols(),
            vec![PeerProtocol::Tcp]
        );
    }

    #[test]
    fn intent_auto_candidates_skip_plain_tcp_by_default() {
        let protocols = intent(TransportMode::Auto).candidate_protocols();
        assert!(protocols.contains(&PeerProtocol::Quic));
        assert!(protocols.contains(&PeerProtocol::TlsTcp));
        assert!(protocols.contains(&PeerProtocol::SshDirect));
        assert!(!protocols.contains(&PeerProtocol::Tcp));
    }

    #[test]
    fn auto_transport_error_lists_candidate_failures() {
        let err = AutoTransportError {
            failures: vec![
                TransportCandidateFailure {
                    protocol: "tls-tcp".to_string(),
                    error: "timeout".to_string(),
                },
                TransportCandidateFailure {
                    protocol: "ssh-direct-tcpip".to_string(),
                    error: "refused".to_string(),
                },
            ],
        };

        assert_eq!(
            err.to_string(),
            "all remote transport candidates failed: tls-tcp: timeout; ssh-direct-tcpip: refused"
        );
    }
}
