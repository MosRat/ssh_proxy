use std::{future::Future, net::SocketAddr, path::PathBuf, pin::Pin};

use anyhow::{Context, Result, bail};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    net::TcpListener,
};
use tokio_rustls::TlsAcceptor;

use crate::{
    peer_transport::{self, PeerProtocol, QuicTransportOptions},
    quic_stream::QuicBiStream,
    remote_helper::{BoxedRemoteStream, opened_remote},
};

pub type ServerFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlainTransportListenerConfig {
    pub addr: SocketAddr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsTransportListenerConfig {
    pub addr: SocketAddr,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub client_ca_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuicTransportListenerConfig {
    pub addr: SocketAddr,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub options: QuicTransportOptions,
}

pub trait PeerTransportServer: Clone + Send + Sync + 'static {
    fn node_name(&self) -> String;
    fn expected_token(&self) -> Option<String>;
    fn shutdown<'a>(&'a self) -> ServerFuture<'a, ()>;
    fn handle_framed_stream<'a>(
        &'a self,
        stream: BoxedRemoteStream,
        peer: SocketAddr,
        protocol: PeerProtocol,
    ) -> ServerFuture<'a, Result<()>>;
    fn handle_quic_connection<'a>(
        &'a self,
        connection: ServerQuicConnection,
        peer: SocketAddr,
    ) -> ServerFuture<'a, Result<()>>;
}

#[derive(Clone)]
pub struct ServerQuicConnection {
    inner: quinn::Connection,
}

impl ServerQuicConnection {
    fn new(inner: quinn::Connection) -> Self {
        Self { inner }
    }

    pub async fn accept_bi(&self) -> Result<QuicBiStream> {
        let (send, recv) = self.inner.accept_bi().await?;
        Ok(QuicBiStream::new(send, recv))
    }

    pub fn close(&self, code: u32, reason: &'static [u8]) {
        self.inner.close(code.into(), reason);
    }
}

pub async fn run_plain_transport_listener<H>(
    config: PlainTransportListenerConfig,
    handler: H,
) -> Result<()>
where
    H: PeerTransportServer,
{
    let listener = TcpListener::bind(config.addr)
        .await
        .with_context(|| format!("failed to bind node transport listener {}", config.addr))?;
    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (mut stream, peer) = accept?;
                let _ = stream.set_nodelay(true);
                let handler = handler.clone();
                tokio::spawn(async move {
                    if verify_token(&mut stream, handler.expected_token().as_deref()).await.is_err() {
                        return;
                    }
                    if peer_transport::server_handshake(
                        &mut stream,
                        handler.node_name(),
                        &[PeerProtocol::SshDirect, PeerProtocol::Tcp],
                    )
                    .await
                    .is_err()
                    {
                        return;
                    }
                    let _ = handler
                        .handle_framed_stream(opened_remote(stream, PeerProtocol::Tcp).stream, peer, PeerProtocol::Tcp)
                        .await;
                });
            }
            _ = handler.shutdown() => break,
        }
    }
    Ok(())
}

pub async fn run_tls_transport_listener<H>(
    config: TlsTransportListenerConfig,
    handler: H,
) -> Result<()>
where
    H: PeerTransportServer,
{
    let certs = peer_transport::load_cert_chain(&config.cert_path)?;
    let key = peer_transport::load_private_key(&config.key_path)?;
    let acceptor = if let Some(client_ca) = config.client_ca_path.as_deref() {
        let client_roots = peer_transport::load_cert_chain(client_ca)?;
        TlsAcceptor::from(peer_transport::tls_server_config_with_client_auth(
            certs,
            key,
            client_roots,
        )?)
    } else {
        TlsAcceptor::from(peer_transport::tls_server_config(certs, key)?)
    };
    let listener = TcpListener::bind(config.addr)
        .await
        .with_context(|| format!("failed to bind node TLS transport listener {}", config.addr))?;
    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, peer) = accept?;
                let _ = stream.set_nodelay(true);
                let acceptor = acceptor.clone();
                let handler = handler.clone();
                tokio::spawn(async move {
                    let Ok(mut stream) = acceptor.accept(stream).await else {
                        return;
                    };
                    if peer_transport::server_handshake(
                        &mut stream,
                        handler.node_name(),
                        &[PeerProtocol::TlsTcp],
                    )
                    .await
                    .is_err()
                    {
                        return;
                    }
                    let _ = handler
                        .handle_framed_stream(opened_remote(stream, PeerProtocol::TlsTcp).stream, peer, PeerProtocol::TlsTcp)
                        .await;
                });
            }
            _ = handler.shutdown() => break,
        }
    }
    Ok(())
}

pub async fn run_quic_transport_listener<H>(
    config: QuicTransportListenerConfig,
    handler: H,
) -> Result<()>
where
    H: PeerTransportServer,
{
    let certs = peer_transport::load_cert_chain(&config.cert_path)?;
    let key = peer_transport::load_private_key(&config.key_path)?;
    let endpoint = quinn::Endpoint::server(
        peer_transport::quic_server_config(certs, key, config.options)?,
        config.addr,
    )
    .with_context(|| {
        format!(
            "failed to bind node QUIC transport listener {}",
            config.addr
        )
    })?;
    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else {
                    break;
                };
                let handler = handler.clone();
                tokio::spawn(async move {
                    let peer = incoming.remote_address();
                    let Ok(connection) = incoming.await else {
                        return;
                    };
                    let _ = handler
                        .handle_quic_connection(ServerQuicConnection::new(connection), peer)
                        .await;
                });
            }
            _ = handler.shutdown() => break,
        }
    }
    Ok(())
}

pub async fn accept_quic_peer_control(
    connection: &ServerQuicConnection,
    node_name: String,
    supported: &[PeerProtocol],
) -> Result<(QuicBiStream, PeerProtocol, peer_transport::PeerHello)> {
    let mut stream = connection
        .accept_bi()
        .await
        .context("node QUIC transport stream accept failed")?;
    let hello = peer_transport::server_handshake(&mut stream, node_name, supported)
        .await
        .context("node QUIC transport handshake failed")?;
    let accepted = peer_transport::select_supported_protocol(&hello.protocols, supported)
        .ok_or_else(|| anyhow::anyhow!("accepted QUIC protocol is missing after handshake"))?;
    Ok((stream, accepted, hello))
}

async fn verify_token<S>(stream: &mut S, expected: Option<&str>) -> Result<()>
where
    S: AsyncRead + Unpin,
{
    if let Some(expected) = expected {
        let mut len = [0_u8; 2];
        stream.read_exact(&mut len).await?;
        let len = u16::from_be_bytes(len) as usize;
        let mut token = vec![0_u8; len];
        stream.read_exact(&mut token).await?;
        if token != expected.as_bytes() {
            bail!("invalid token");
        }
    }
    Ok(())
}
