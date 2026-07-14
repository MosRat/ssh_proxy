use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use anyhow::{Result, anyhow};
use ssh_proxy_transport::{
    peer_transport::PeerProtocol,
    remote_helper::BoxedRemoteStream,
    server::{
        PeerTransportServer, PlainTransportListenerConfig, QuicTransportListenerConfig,
        ServerFuture, ServerQuicConnection, TlsTransportListenerConfig,
    },
};

use crate::remote;

use super::{NodeManager, quic_transport};

pub(super) async fn run_transport_listener(
    addr: SocketAddr,
    manager: Arc<NodeManager>,
) -> Result<()> {
    ssh_proxy_transport::server::run_plain_transport_listener(
        PlainTransportListenerConfig { addr },
        NodeTransportHandler::new(manager),
    )
    .await
}

pub(super) async fn run_tls_transport_listener(
    addr: SocketAddr,
    manager: Arc<NodeManager>,
) -> Result<()> {
    let cert_path = manager
        .tls_cert
        .clone()
        .ok_or_else(|| anyhow!("--tls-cert is required with --tls-transport"))?;
    let key_path = manager
        .tls_key
        .clone()
        .ok_or_else(|| anyhow!("--tls-key is required with --tls-transport"))?;
    ssh_proxy_transport::server::run_tls_transport_listener(
        TlsTransportListenerConfig {
            addr,
            cert_path,
            key_path,
            client_ca_path: manager.tls_client_ca.clone(),
        },
        NodeTransportHandler::new(manager),
    )
    .await
}

pub(super) async fn run_quic_transport_listener(
    addr: SocketAddr,
    manager: Arc<NodeManager>,
) -> Result<()> {
    let cert_path = manager
        .tls_cert
        .clone()
        .ok_or_else(|| anyhow!("--tls-cert is required with --quic-transport"))?;
    let key_path = manager
        .tls_key
        .clone()
        .ok_or_else(|| anyhow!("--tls-key is required with --quic-transport"))?;
    ssh_proxy_transport::server::run_quic_transport_listener(
        QuicTransportListenerConfig {
            addr,
            cert_path,
            key_path,
            options: manager.quic_options,
        },
        NodeTransportHandler::new(manager),
    )
    .await
}

#[derive(Clone)]
struct NodeTransportHandler {
    manager: Arc<NodeManager>,
}

impl NodeTransportHandler {
    fn new(manager: Arc<NodeManager>) -> Self {
        Self { manager }
    }
}

impl PeerTransportServer for NodeTransportHandler {
    fn node_name(&self) -> String {
        self.manager.name.clone()
    }

    fn expected_token(&self) -> Option<String> {
        self.manager.token_value()
    }

    fn shutdown<'a>(&'a self) -> ServerFuture<'a, ()> {
        Box::pin(async move {
            self.manager.shutdown_notified().await;
        })
    }

    fn handle_framed_stream<'a>(
        &'a self,
        stream: BoxedRemoteStream,
        peer: SocketAddr,
        _protocol: PeerProtocol,
    ) -> ServerFuture<'a, Result<()>> {
        Box::pin(async move {
            let _guard = ActiveTransportGuard::new(
                &self.manager.total_transports,
                &self.manager.active_transports,
            );
            let (reader, writer) = tokio::io::split(stream);
            remote::run_transport(reader, writer)
                .await
                .map_err(|err| anyhow!("node framed transport failed for {peer}: {err:#}"))
        })
    }

    fn handle_quic_connection<'a>(
        &'a self,
        connection: ServerQuicConnection,
        peer: SocketAddr,
    ) -> ServerFuture<'a, Result<()>> {
        Box::pin(async move {
            quic_transport::handle_connection(connection, peer, self.manager.clone()).await;
            Ok(())
        })
    }
}

struct ActiveTransportGuard<'a> {
    active: &'a AtomicU32,
}

impl<'a> ActiveTransportGuard<'a> {
    fn new(total: &'a AtomicU32, active: &'a AtomicU32) -> Self {
        total.fetch_add(1, Ordering::Relaxed);
        active.fetch_add(1, Ordering::Relaxed);
        Self { active }
    }
}

impl Drop for ActiveTransportGuard<'_> {
    fn drop(&mut self) {
        self.active
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_sub(1)
            })
            .ok();
    }
}
