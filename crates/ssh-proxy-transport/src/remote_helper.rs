use std::fmt;

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::peer_transport::PeerProtocol;

pub trait RemoteStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> RemoteStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

pub type BoxedRemoteStream = Box<dyn RemoteStream>;

pub struct OpenedRemoteHelper {
    pub stream: BoxedRemoteStream,
    pub protocol: PeerProtocol,
    pub timings: RemoteHelperTimings,
}

#[derive(Debug, Clone, Copy, Default)]
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
