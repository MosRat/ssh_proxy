use std::{
    net::SocketAddr,
    sync::{Arc, atomic::Ordering},
};

use anyhow::{Context, Result, anyhow, bail};
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
};
use tokio_rustls::TlsAcceptor;
use tracing::{info, warn};

use crate::{peer_transport, remote};

use super::{NodeManager, quic_transport};

pub(super) async fn run_transport_listener(
    addr: SocketAddr,
    manager: Arc<NodeManager>,
) -> Result<()> {
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind node transport listener {addr}"))?;
    info!(%addr, "node framed transport listening");
    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (mut stream, peer) = accept?;
                let _ = stream.set_nodelay(true);
                let manager = manager.clone();
                tokio::spawn(async move {
                    let expected_token = manager.token_value();
                    if let Err(err) = verify_token(&mut stream, expected_token.as_deref()).await {
                        warn!(%peer, error = %err, "node transport auth failed");
                        return;
                    }
                    let hello = match peer_transport::server_handshake(
                        &mut stream,
                        manager.name.clone(),
                        &[peer_transport::PeerProtocol::SshDirect, peer_transport::PeerProtocol::Tcp],
                    )
                    .await
                    {
                        Ok(hello) => hello,
                        Err(err) => {
                            warn!(%peer, error = %err, "node transport handshake failed");
                            return;
                        }
                    };
                    info!(
                        %peer,
                        remote_node = %hello.node,
                        protocols = ?hello.protocols,
                        "node transport handshake completed"
                    );
                    manager.total_transports.fetch_add(1, Ordering::Relaxed);
                    manager.active_transports.fetch_add(1, Ordering::Relaxed);
                    let (reader, writer) = tokio::io::split(stream);
                    if let Err(err) = remote::run_transport(reader, writer).await {
                        warn!(%peer, error = %err, "node transport failed");
                    }
                    manager.active_transports.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(1)).ok();
                });
            }
            _ = manager.shutdown_notified() => break,
        }
    }
    Ok(())
}

pub(super) async fn run_tls_transport_listener(
    addr: SocketAddr,
    manager: Arc<NodeManager>,
) -> Result<()> {
    let cert_path = manager
        .tls_cert
        .as_deref()
        .ok_or_else(|| anyhow!("--tls-cert is required with --tls-transport"))?;
    let key_path = manager
        .tls_key
        .as_deref()
        .ok_or_else(|| anyhow!("--tls-key is required with --tls-transport"))?;
    let certs = peer_transport::load_cert_chain(cert_path)?;
    let key = peer_transport::load_private_key(key_path)?;
    let acceptor = if let Some(client_ca) = manager.tls_client_ca.as_deref() {
        let client_roots = peer_transport::load_cert_chain(client_ca)?;
        TlsAcceptor::from(peer_transport::tls_server_config_with_client_auth(
            certs,
            key,
            client_roots,
        )?)
    } else {
        TlsAcceptor::from(peer_transport::tls_server_config(certs, key)?)
    };
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind node TLS transport listener {addr}"))?;
    info!(%addr, "node TLS framed transport listening");
    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, peer) = accept?;
                let _ = stream.set_nodelay(true);
                let acceptor = acceptor.clone();
                let manager = manager.clone();
                tokio::spawn(async move {
                    let mut stream = match acceptor.accept(stream).await {
                        Ok(stream) => stream,
                        Err(err) => {
                            warn!(%peer, error = %err, "node TLS transport accept failed");
                            return;
                        }
                    };
                    let hello = match peer_transport::server_handshake(
                        &mut stream,
                        manager.name.clone(),
                        &[peer_transport::PeerProtocol::TlsTcp],
                    )
                    .await
                    {
                        Ok(hello) => hello,
                        Err(err) => {
                            warn!(%peer, error = %err, "node TLS transport handshake failed");
                            return;
                        }
                    };
                    info!(
                        %peer,
                        remote_node = %hello.node,
                        protocols = ?hello.protocols,
                        "node TLS transport handshake completed"
                    );
                    manager.total_transports.fetch_add(1, Ordering::Relaxed);
                    manager.active_transports.fetch_add(1, Ordering::Relaxed);
                    let (reader, writer) = tokio::io::split(stream);
                    if let Err(err) = remote::run_transport(reader, writer).await {
                        warn!(%peer, error = %err, "node TLS transport failed");
                    }
                    manager.active_transports.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(1)).ok();
                });
            }
            _ = manager.shutdown_notified() => break,
        }
    }
    Ok(())
}

pub(super) async fn run_quic_transport_listener(
    addr: SocketAddr,
    manager: Arc<NodeManager>,
) -> Result<()> {
    let cert_path = manager
        .tls_cert
        .as_deref()
        .ok_or_else(|| anyhow!("--tls-cert is required with --quic-transport"))?;
    let key_path = manager
        .tls_key
        .as_deref()
        .ok_or_else(|| anyhow!("--tls-key is required with --quic-transport"))?;
    let certs = peer_transport::load_cert_chain(cert_path)?;
    let key = peer_transport::load_private_key(key_path)?;
    let endpoint = quinn::Endpoint::server(
        peer_transport::quic_server_config(certs, key, manager.quic_options)?,
        addr,
    )
    .with_context(|| format!("failed to bind node QUIC transport listener {addr}"))?;
    info!(%addr, "node QUIC transport listening");
    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else {
                    break;
                };
                let manager = manager.clone();
                tokio::spawn(async move {
                    let peer = incoming.remote_address();
                    let connection = match incoming.await {
                        Ok(connection) => connection,
                        Err(err) => {
                            warn!(%peer, error = %err, "node QUIC transport connect failed");
                            return;
                        }
                    };
                    quic_transport::handle_connection(connection, peer, manager).await;
                });
            }
            _ = manager.shutdown_notified() => break,
        }
    }
    Ok(())
}

async fn verify_token(stream: &mut TcpStream, expected: Option<&str>) -> Result<()> {
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
