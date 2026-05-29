use std::{future::Future, net::SocketAddr, pin::Pin, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use ssh_proxy_core::model::TransportMode;
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    time,
};
use tokio_rustls::{TlsConnector, rustls::pki_types::ServerName};

use crate::{
    peer_transport::{self, PeerProtocol},
    quic_stream::QuicBiStream,
    remote_helper::{
        AutoTransportError, BoxedRemoteStream, OpenedRemoteHelper, RemoteHelperOpenIntent,
        RemoteHelperTimings, TransportCandidateFailure, opened_remote,
    },
};

pub type RemoteHelperFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait SshDirectConnector {
    fn direct_tcpip_stream<'a>(
        &'a mut self,
        remote_tcp: SocketAddr,
    ) -> RemoteHelperFuture<'a, Result<BoxedRemoteStream>>;

    fn exec_helper_stream<'a>(
        &'a mut self,
        intent: &'a RemoteHelperOpenIntent,
    ) -> RemoteHelperFuture<'a, Result<BoxedRemoteStream>>;
}

pub async fn open_remote_helper<C>(
    intent: &RemoteHelperOpenIntent,
    local_node_name: &str,
    ssh: &mut C,
) -> Result<OpenedRemoteHelper>
where
    C: SshDirectConnector + Send,
{
    match intent.transport {
        TransportMode::SshNative => {
            bail!("ssh-native is a proxy data-plane mode and does not open a SPX remote helper")
        }
        TransportMode::QuicNative => {
            bail!(
                "quic-native is a native per-flow data-plane mode handled by the proxy runtime; it does not open a SPX remote helper"
            )
        }
        TransportMode::Quic => open_quic_transport(intent, local_node_name)
            .await
            .with_context(|| {
                format!(
                    "failed to open remote QUIC peer transport at {:?}",
                    intent.remote_quic
                )
            }),
        TransportMode::TlsTcp => open_tls_transport(intent, local_node_name)
            .await
            .with_context(|| {
                format!(
                    "failed to open remote TLS peer transport at {:?}",
                    intent.remote_tls
                )
            }),
        TransportMode::PlainTcp => open_plain_tcp_transport(intent, local_node_name)
            .await
            .with_context(|| {
                format!(
                    "failed to open remote plain TCP peer transport {}",
                    intent.remote_tcp
                )
            }),
        TransportMode::Tcp => open_ssh_direct_transport(intent, local_node_name, ssh)
            .await
            .with_context(|| {
                format!(
                    "failed to open remote persistent helper at {} through SSH",
                    intent.remote_tcp
                )
            }),
        TransportMode::Exec => open_exec_helper(intent, ssh).await,
        TransportMode::Auto => open_auto_transport(intent, local_node_name, ssh).await,
    }
}

async fn open_auto_transport<C>(
    intent: &RemoteHelperOpenIntent,
    local_node_name: &str,
    ssh: &mut C,
) -> Result<OpenedRemoteHelper>
where
    C: SshDirectConnector + Send,
{
    let mut failures = Vec::new();
    for protocol in intent.candidate_protocols() {
        let result = match protocol {
            PeerProtocol::SshNative | PeerProtocol::QuicNative => continue,
            PeerProtocol::Quic => {
                if intent.remote_quic.is_none() {
                    continue;
                }
                open_quic_transport(intent, local_node_name).await
            }
            PeerProtocol::TlsTcp => {
                if intent.remote_tls.is_none() {
                    continue;
                }
                open_tls_transport(intent, local_node_name).await
            }
            PeerProtocol::Tcp => {
                if !intent.allow_plain_tcp {
                    continue;
                }
                open_plain_tcp_transport(intent, local_node_name).await
            }
            PeerProtocol::SshDirect => {
                open_ssh_direct_transport(intent, local_node_name, ssh).await
            }
            PeerProtocol::SshExec => open_exec_helper(intent, ssh).await,
        };
        match result {
            Ok(opened) => return Ok(opened),
            Err(err) => failures.push(TransportCandidateFailure {
                protocol: protocol.to_string(),
                error: format!("{err:#}"),
            }),
        }
    }
    Err(anyhow!(AutoTransportError { failures }))
}

async fn open_ssh_direct_transport<C>(
    intent: &RemoteHelperOpenIntent,
    local_node_name: &str,
    ssh: &mut C,
) -> Result<OpenedRemoteHelper>
where
    C: SshDirectConnector + Send,
{
    let channel_started = std::time::Instant::now();
    let mut stream = with_connect_timeout(
        intent.connect_timeout_secs,
        format!("SSH direct-tcpip to {}", intent.remote_tcp),
        ssh.direct_tcpip_stream(intent.remote_tcp),
    )
    .await??;
    let ssh_direct_channel_open_latency_ms = duration_millis(channel_started.elapsed());
    send_transport_token(&mut stream, intent.remote_token.as_deref()).await?;
    let handshake_started = std::time::Instant::now();
    peer_transport::client_handshake(
        &mut stream,
        local_node_name.to_string(),
        PeerProtocol::SshDirect,
    )
    .await?;
    let spx_peer_handshake_latency_ms = duration_millis(handshake_started.elapsed());
    Ok(OpenedRemoteHelper {
        stream,
        protocol: PeerProtocol::SshDirect,
        timings: RemoteHelperTimings {
            ssh_direct_channel_open_latency_ms: Some(ssh_direct_channel_open_latency_ms),
            spx_peer_handshake_latency_ms: Some(spx_peer_handshake_latency_ms),
        },
    })
}

async fn open_exec_helper<C>(
    intent: &RemoteHelperOpenIntent,
    ssh: &mut C,
) -> Result<OpenedRemoteHelper>
where
    C: SshDirectConnector + Send,
{
    Ok(OpenedRemoteHelper {
        stream: ssh.exec_helper_stream(intent).await?,
        protocol: PeerProtocol::SshExec,
        timings: RemoteHelperTimings::default(),
    })
}

async fn open_plain_tcp_transport(
    intent: &RemoteHelperOpenIntent,
    local_node_name: &str,
) -> Result<OpenedRemoteHelper> {
    let mut stream = connect_tcp_with_timeout(
        intent.remote_tcp,
        intent.connect_timeout_secs,
        "direct TCP transport",
    )
    .await
    .with_context(|| {
        format!(
            "failed to connect direct TCP transport {}",
            intent.remote_tcp
        )
    })?;
    let _ = stream.set_nodelay(true);
    send_transport_token(&mut stream, intent.remote_token.as_deref()).await?;
    peer_transport::client_handshake(&mut stream, local_node_name.to_string(), PeerProtocol::Tcp)
        .await?;
    Ok(opened_remote(stream, PeerProtocol::Tcp))
}

async fn open_quic_transport(
    intent: &RemoteHelperOpenIntent,
    local_node_name: &str,
) -> Result<OpenedRemoteHelper> {
    let addr = intent
        .remote_quic
        .ok_or_else(|| anyhow!("--remote-quic is required for quic transport"))?;
    let ca = intent
        .remote_ca
        .as_deref()
        .ok_or_else(|| anyhow!("--remote-ca is required for quic transport"))?;
    if intent.remote_client_cert.is_some() || intent.remote_client_key.is_some() {
        bail!("QUIC mTLS is not implemented yet; use tls-tcp for mTLS");
    }
    let roots = peer_transport::load_cert_chain(ca)?;
    let mut endpoint = quinn::Endpoint::client(SocketAddr::from(([0, 0, 0, 0], 0)))
        .context("failed to create QUIC client endpoint")?;
    endpoint.set_default_client_config(peer_transport::quic_client_config(roots, intent.quic)?);
    let connecting = endpoint
        .connect(addr, &intent.remote_name)
        .context("failed to create QUIC connect request")?;
    let connection = time::timeout(
        Duration::from_secs(intent.connect_timeout_secs.max(1)),
        connecting,
    )
    .await
    .with_context(|| {
        format!(
            "remote QUIC transport {addr} timed out after {}s",
            intent.connect_timeout_secs.max(1)
        )
    })?
    .with_context(|| format!("failed to connect remote QUIC transport {addr}"))?;
    let (send, recv) = with_connect_timeout(
        intent.connect_timeout_secs,
        format!("open QUIC peer stream at {addr}"),
        connection.open_bi(),
    )
    .await?
    .context("failed to open QUIC peer stream")?;
    let mut stream = QuicBiStream::with_lifetime(send, recv, connection, endpoint);
    peer_transport::client_handshake(&mut stream, local_node_name.to_string(), PeerProtocol::Quic)
        .await?;
    Ok(opened_remote(stream, PeerProtocol::Quic))
}

async fn open_tls_transport(
    intent: &RemoteHelperOpenIntent,
    local_node_name: &str,
) -> Result<OpenedRemoteHelper> {
    let addr = intent
        .remote_tls
        .ok_or_else(|| anyhow!("--remote-tls is required for tls-tcp transport"))?;
    let ca = intent
        .remote_ca
        .as_deref()
        .ok_or_else(|| anyhow!("--remote-ca is required for tls-tcp transport"))?;
    let roots = peer_transport::load_cert_chain(ca)?;
    let config = match (
        intent.remote_client_cert.as_deref(),
        intent.remote_client_key.as_deref(),
    ) {
        (Some(cert), Some(key)) => peer_transport::tls_client_config_with_client_auth(
            roots,
            peer_transport::load_cert_chain(cert)?,
            peer_transport::load_private_key(key)?,
        )?,
        (None, None) => peer_transport::tls_client_config(roots)?,
        _ => bail!("--remote-client-cert and --remote-client-key must be used together"),
    };
    let stream =
        connect_tcp_with_timeout(addr, intent.connect_timeout_secs, "remote TLS transport")
            .await
            .with_context(|| format!("failed to connect remote TLS transport {addr}"))?;
    let _ = stream.set_nodelay(true);
    let server_name = ServerName::try_from(intent.remote_name.clone())
        .context("invalid --remote-name for TLS server name")?;
    let connector = TlsConnector::from(config);
    let mut stream = with_connect_timeout(
        intent.connect_timeout_secs,
        format!("remote TLS handshake with {addr}"),
        connector.connect(server_name, stream),
    )
    .await?
    .context("failed to establish remote TLS transport")?;
    peer_transport::client_handshake(
        &mut stream,
        local_node_name.to_string(),
        PeerProtocol::TlsTcp,
    )
    .await?;
    Ok(opened_remote(stream, PeerProtocol::TlsTcp))
}

async fn send_transport_token<S>(stream: &mut S, token: Option<&str>) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    if let Some(token) = token {
        let bytes = token.as_bytes();
        if bytes.len() > u16::MAX as usize {
            bail!("remote token is too long");
        }
        stream
            .write_all(&(bytes.len() as u16).to_be_bytes())
            .await?;
        stream.write_all(bytes).await?;
        stream.flush().await?;
    }
    Ok(())
}

async fn connect_tcp_with_timeout(
    addr: SocketAddr,
    timeout_secs: u64,
    label: &'static str,
) -> Result<TcpStream> {
    time::timeout(
        Duration::from_secs(timeout_secs.max(1)),
        TcpStream::connect(addr),
    )
    .await
    .with_context(|| format!("{label} {addr} timed out after {}s", timeout_secs.max(1)))?
    .with_context(|| format!("failed to connect {label} {addr}"))
}

async fn with_connect_timeout<F, T>(
    timeout_secs: u64,
    label: impl Into<String>,
    operation: F,
) -> Result<T>
where
    F: Future<Output = T>,
{
    let timeout_secs = timeout_secs.max(1);
    let label = label.into();
    time::timeout(Duration::from_secs(timeout_secs), operation)
        .await
        .with_context(|| format!("{label} timed out after {timeout_secs}s"))
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}
