use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result};
use tokio::time;
use tokio_rustls::rustls::pki_types::CertificateDer;

use crate::{
    peer_transport::{self, QuicTransportOptions},
    quic_stream::QuicBiStream,
};

#[derive(Clone)]
pub struct ClientQuicConnection {
    inner: quinn::Connection,
    _endpoint: quinn::Endpoint,
}

impl ClientQuicConnection {
    pub async fn open_bi(
        &self,
        timeout: Duration,
        label: impl Into<String>,
    ) -> Result<QuicBiStream> {
        let timeout = timeout.max(Duration::from_millis(1));
        let label = label.into();
        let (send, recv) = time::timeout(timeout, self.inner.open_bi())
            .await
            .with_context(|| format!("{label} timed out after {} ms", timeout.as_millis()))?
            .with_context(|| format!("failed to {label}"))?;
        Ok(QuicBiStream::with_connection(
            send,
            recv,
            self.inner.clone(),
        ))
    }
}

pub async fn connect_client(
    addr: SocketAddr,
    server_name: &str,
    roots: Vec<CertificateDer<'static>>,
    options: QuicTransportOptions,
    timeout: Duration,
    label: impl Into<String>,
) -> Result<ClientQuicConnection> {
    let timeout = timeout.max(Duration::from_millis(1));
    let label = label.into();
    let mut endpoint = quinn::Endpoint::client(SocketAddr::from(([0, 0, 0, 0], 0)))
        .context("failed to create QUIC client endpoint")?;
    endpoint.set_default_client_config(peer_transport::quic_client_config(roots, options)?);
    let connecting = endpoint
        .connect(addr, server_name)
        .context("failed to create QUIC connect request")?;
    let connection = time::timeout(timeout, connecting)
        .await
        .with_context(|| format!("{label} timed out after {} ms", timeout.as_millis()))?
        .with_context(|| format!("failed to {label}"))?;
    Ok(ClientQuicConnection {
        inner: connection,
        _endpoint: endpoint,
    })
}
