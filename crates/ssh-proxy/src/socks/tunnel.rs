use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use ssh_proxy_transport::proxy::http::{HttpRequest, HttpRequestKind, write_http_error};
use tokio::net::TcpStream;

use crate::{controller, quic_native, ssh_native};

pub(super) enum TunnelBackend {
    Spx(Arc<controller::SharedState>),
    SshNative(Arc<ssh_native::State>),
    QuicNative {
        state: Arc<quic_native::State>,
        kind: quic_native::TargetKind,
    },
}

impl TunnelBackend {
    pub(super) fn spx(state: Arc<controller::SharedState>) -> Self {
        Self::Spx(state)
    }

    pub(super) fn ssh_native(state: Arc<ssh_native::State>) -> Self {
        Self::SshNative(state)
    }

    pub(super) fn quic_native(
        state: Arc<quic_native::State>,
        kind: quic_native::TargetKind,
    ) -> Self {
        Self::QuicNative { state, kind }
    }
}

pub(super) enum TunnelResponse {
    None,
    SocksSuccess,
    HttpConnect,
}

pub(super) async fn handle_http_proxy(
    mut stream: TcpStream,
    peer: SocketAddr,
    backend: TunnelBackend,
) -> Result<()> {
    let request = match HttpRequest::read_from(&mut stream).await {
        Ok(request) => request,
        Err(err) => {
            let _ = write_http_error(&mut stream, 400, "Bad Request").await;
            return Err(err);
        }
    };
    match request.kind {
        HttpRequestKind::Connect { host, port } => {
            open_tunnel(
                stream,
                peer,
                host,
                port,
                backend,
                TunnelResponse::HttpConnect,
                Vec::new(),
            )
            .await
        }
        HttpRequestKind::Forward {
            host,
            port,
            request,
        } => {
            open_tunnel(
                stream,
                peer,
                host,
                port,
                backend,
                TunnelResponse::None,
                request,
            )
            .await
        }
    }
}

pub(super) async fn open_tunnel(
    stream: TcpStream,
    peer: SocketAddr,
    host: String,
    port: u16,
    backend: TunnelBackend,
    response: TunnelResponse,
    initial_remote: Vec<u8>,
) -> Result<()> {
    match backend {
        TunnelBackend::Spx(state) => {
            super::open_tunnel(stream, peer, host, port, state, response, initial_remote).await
        }
        TunnelBackend::SshNative(state) => {
            super::open_tunnel_ssh_native(stream, peer, host, port, state, response, initial_remote)
                .await
        }
        TunnelBackend::QuicNative { state, kind } => {
            super::open_tunnel_quic_native(
                stream,
                peer,
                host,
                port,
                kind,
                state,
                response,
                initial_remote,
            )
            .await
        }
    }
}
