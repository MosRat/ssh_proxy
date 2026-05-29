use std::{future::Future, net::SocketAddr, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    time,
};
use tokio_rustls::{TlsConnector, rustls::pki_types::ServerName};
use tracing::{info, warn};

use crate::{cli, peer_transport, quic_stream, ssh_client};

use super::helper::{
    HelperCapability, ensure_helper, remote_reverse_socks_command, remote_stdio_command,
};

pub trait RemoteStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> RemoteStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

pub type BoxedRemoteStream = Box<dyn RemoteStream>;

pub(crate) struct OpenedRemoteHelper {
    pub(crate) stream: BoxedRemoteStream,
    pub(crate) protocol: peer_transport::PeerProtocol,
    pub(crate) timings: RemoteHelperTimings,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RemoteHelperTimings {
    pub(crate) ssh_direct_channel_open_latency_ms: Option<u64>,
    pub(crate) spx_peer_handshake_latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TransportCandidateFailure {
    pub(crate) protocol: String,
    pub(crate) error: String,
}

#[derive(Debug, Clone)]
pub(crate) struct AutoTransportError {
    pub(crate) failures: Vec<TransportCandidateFailure>,
}

impl std::fmt::Display for AutoTransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

pub async fn open_remote_helper(args: &cli::ProxyArgs) -> Result<OpenedRemoteHelper> {
    match args.remote_transport {
        cli::RemoteTransport::SshNative => {
            bail!("ssh-native is a proxy data-plane mode and does not open a SPX remote helper")
        }
        cli::RemoteTransport::QuicNative => {
            bail!(
                "quic-native is a native per-flow data-plane mode handled by the proxy runtime; it does not open a SPX remote helper"
            )
        }
        cli::RemoteTransport::Quic => open_quic_transport(args).await.with_context(|| {
            format!(
                "failed to open remote QUIC peer transport at {:?}",
                args.remote_quic
            )
        }),
        cli::RemoteTransport::TlsTcp => open_tls_transport(args).await.with_context(|| {
            format!(
                "failed to open remote TLS peer transport at {:?}",
                args.remote_tls
            )
        }),
        cli::RemoteTransport::PlainTcp => open_plain_tcp_transport(args).await.with_context(|| {
            format!(
                "failed to open remote plain TCP peer transport {}",
                args.remote_tcp
            )
        }),
        cli::RemoteTransport::Tcp => {
            let client = ssh_client::Client::connect_proxy_args(args).await?;
            open_persistent_transport(args, &client, peer_transport::PeerProtocol::SshDirect)
                .await
                .with_context(|| {
                    format!(
                        "failed to open remote persistent helper at {} through SSH",
                        args.remote_tcp
                    )
                })
        }
        cli::RemoteTransport::Exec => {
            let client = ssh_client::Client::connect_proxy_args(args).await?;
            open_exec_helper(args, &client).await
        }
        cli::RemoteTransport::Auto => open_auto_transport(args).await,
    }
}

async fn open_auto_transport(args: &cli::ProxyArgs) -> Result<OpenedRemoteHelper> {
    let hints = peer_transport::NetworkHints {
        peer_addr: args
            .remote_quic
            .or(args.remote_tls)
            .or(Some(args.remote_tcp)),
        ssh_available: true,
        allow_plain_tcp: args.allow_plain_tcp,
        prefer_low_latency: true,
    };
    let mut failures = Vec::new();
    let mut ssh_client = None;
    for candidate in peer_transport::implemented_auto_candidates(&hints) {
        let result = match candidate.protocol {
            peer_transport::PeerProtocol::SshNative => continue,
            peer_transport::PeerProtocol::QuicNative => continue,
            peer_transport::PeerProtocol::Quic => {
                if args.remote_quic.is_none() {
                    continue;
                }
                open_quic_transport(args).await
            }
            peer_transport::PeerProtocol::TlsTcp => {
                if args.remote_tls.is_none() {
                    continue;
                }
                open_tls_transport(args).await
            }
            peer_transport::PeerProtocol::Tcp => {
                if !args.allow_plain_tcp {
                    continue;
                }
                open_plain_tcp_transport(args).await
            }
            peer_transport::PeerProtocol::SshDirect => {
                if ssh_client.is_none() {
                    ssh_client = Some(match ssh_client::Client::connect_proxy_args(args).await {
                        Ok(client) => client,
                        Err(err) => {
                            failures.push(TransportCandidateFailure {
                                protocol: candidate.protocol.to_string(),
                                error: format!("{err:#}"),
                            });
                            continue;
                        }
                    });
                }
                let client = ssh_client.as_ref().expect("ssh client just inserted");
                open_persistent_transport(args, &client, candidate.protocol).await
            }
            peer_transport::PeerProtocol::SshExec => {
                if ssh_client.is_none() {
                    ssh_client = Some(match ssh_client::Client::connect_proxy_args(args).await {
                        Ok(client) => client,
                        Err(err) => {
                            failures.push(TransportCandidateFailure {
                                protocol: candidate.protocol.to_string(),
                                error: format!("{err:#}"),
                            });
                            continue;
                        }
                    });
                }
                let client = ssh_client.as_ref().expect("ssh client just inserted");
                open_exec_helper(args, &client).await
            }
        };
        match result {
            Ok(opened) => return Ok(opened),
            Err(err) => {
                warn!(protocol = %candidate.protocol, error = %err, "remote transport candidate failed");
                failures.push(TransportCandidateFailure {
                    protocol: candidate.protocol.to_string(),
                    error: format!("{err:#}"),
                });
            }
        }
    }
    Err(anyhow!(AutoTransportError { failures }))
}

async fn open_persistent_transport(
    args: &cli::ProxyArgs,
    client: &ssh_client::Client,
    protocol: peer_transport::PeerProtocol,
) -> Result<OpenedRemoteHelper> {
    let channel_started = std::time::Instant::now();
    let mut stream = with_connect_timeout(
        args.connect_timeout_secs,
        format!("SSH direct-tcpip to {}", args.remote_tcp),
        client.direct_tcpip_stream(args.remote_tcp.ip().to_string(), args.remote_tcp.port()),
    )
    .await??;
    let ssh_direct_channel_open_latency_ms = duration_millis(channel_started.elapsed());
    send_transport_token(&mut stream, args.remote_token.as_deref()).await?;
    let handshake_started = std::time::Instant::now();
    let welcome =
        peer_transport::client_handshake(&mut stream, local_node_name(), protocol).await?;
    let spx_peer_handshake_latency_ms = duration_millis(handshake_started.elapsed());
    info!(
        remote_tcp = %args.remote_tcp,
        remote_node = %welcome.node,
        protocol = %protocol,
        ssh_direct_channel_open_latency_ms,
        spx_peer_handshake_latency_ms,
        "connected to persistent remote helper through SSH direct-tcpip"
    );
    Ok(OpenedRemoteHelper {
        stream: Box::new(stream),
        protocol,
        timings: RemoteHelperTimings {
            ssh_direct_channel_open_latency_ms: Some(ssh_direct_channel_open_latency_ms),
            spx_peer_handshake_latency_ms: Some(spx_peer_handshake_latency_ms),
        },
    })
}

async fn open_plain_tcp_transport(args: &cli::ProxyArgs) -> Result<OpenedRemoteHelper> {
    let mut stream = connect_tcp_with_timeout(
        args.remote_tcp,
        args.connect_timeout_secs,
        "direct TCP transport",
    )
    .await
    .with_context(|| format!("failed to connect direct TCP transport {}", args.remote_tcp))?;
    let _ = stream.set_nodelay(true);
    send_transport_token(&mut stream, args.remote_token.as_deref()).await?;
    let welcome = peer_transport::client_handshake(
        &mut stream,
        local_node_name(),
        peer_transport::PeerProtocol::Tcp,
    )
    .await?;
    info!(
        remote_tcp = %args.remote_tcp,
        remote_node = %welcome.node,
        protocol = %peer_transport::PeerProtocol::Tcp,
        "connected to direct plain TCP peer transport"
    );
    Ok(opened_remote(stream, peer_transport::PeerProtocol::Tcp))
}

async fn open_quic_transport(args: &cli::ProxyArgs) -> Result<OpenedRemoteHelper> {
    let addr = args
        .remote_quic
        .ok_or_else(|| anyhow!("--remote-quic is required for quic transport"))?;
    let ca = args
        .remote_ca
        .as_deref()
        .ok_or_else(|| anyhow!("--remote-ca is required for quic transport"))?;
    if args.remote_client_cert.is_some() || args.remote_client_key.is_some() {
        bail!("QUIC mTLS is not implemented yet; use tls-tcp for mTLS");
    }
    let roots = peer_transport::load_cert_chain(ca)?;
    let mut endpoint = quinn::Endpoint::client(SocketAddr::from(([0, 0, 0, 0], 0)))
        .context("failed to create QUIC client endpoint")?;
    endpoint.set_default_client_config(peer_transport::quic_client_config(
        roots,
        peer_transport::QuicTransportOptions::new(
            args.quic_max_bidi_streams,
            args.quic_stream_receive_window,
            args.quic_receive_window,
            args.quic_keep_alive_interval_secs,
            args.quic_idle_timeout_secs,
        )?,
    )?);
    let connecting = endpoint
        .connect(addr, &args.remote_name)
        .context("failed to create QUIC connect request")?;
    let connection = time::timeout(
        Duration::from_secs(args.connect_timeout_secs.max(1)),
        connecting,
    )
    .await
    .with_context(|| {
        format!(
            "remote QUIC transport {addr} timed out after {}s",
            args.connect_timeout_secs.max(1)
        )
    })?
    .with_context(|| format!("failed to connect remote QUIC transport {addr}"))?;
    let (send, recv) = with_connect_timeout(
        args.connect_timeout_secs,
        format!("open QUIC peer stream at {addr}"),
        connection.open_bi(),
    )
    .await?
    .context("failed to open QUIC peer stream")?;
    let mut stream = quic_stream::QuicBiStream::with_lifetime(send, recv, connection, endpoint);
    let welcome = peer_transport::client_handshake(
        &mut stream,
        local_node_name(),
        peer_transport::PeerProtocol::Quic,
    )
    .await?;
    info!(
        remote_quic = %addr,
        remote_node = %welcome.node,
        protocol = %peer_transport::PeerProtocol::Quic,
        "connected to direct QUIC peer transport"
    );
    Ok(opened_remote(stream, peer_transport::PeerProtocol::Quic))
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

async fn open_tls_transport(args: &cli::ProxyArgs) -> Result<OpenedRemoteHelper> {
    let addr = args
        .remote_tls
        .ok_or_else(|| anyhow!("--remote-tls is required for tls-tcp transport"))?;
    let ca = args
        .remote_ca
        .as_deref()
        .ok_or_else(|| anyhow!("--remote-ca is required for tls-tcp transport"))?;
    let roots = peer_transport::load_cert_chain(ca)?;
    let config = match (
        args.remote_client_cert.as_deref(),
        args.remote_client_key.as_deref(),
    ) {
        (Some(cert), Some(key)) => peer_transport::tls_client_config_with_client_auth(
            roots,
            peer_transport::load_cert_chain(cert)?,
            peer_transport::load_private_key(key)?,
        )?,
        (None, None) => peer_transport::tls_client_config(roots)?,
        _ => bail!("--remote-client-cert and --remote-client-key must be used together"),
    };
    let stream = connect_tcp_with_timeout(addr, args.connect_timeout_secs, "remote TLS transport")
        .await
        .with_context(|| format!("failed to connect remote TLS transport {addr}"))?;
    let _ = stream.set_nodelay(true);
    let server_name = ServerName::try_from(args.remote_name.clone())
        .context("invalid --remote-name for TLS server name")?;
    let connector = TlsConnector::from(config);
    let mut stream = with_connect_timeout(
        args.connect_timeout_secs,
        format!("remote TLS handshake with {addr}"),
        connector.connect(server_name, stream),
    )
    .await?
    .context("failed to establish remote TLS transport")?;
    let welcome = peer_transport::client_handshake(
        &mut stream,
        local_node_name(),
        peer_transport::PeerProtocol::TlsTcp,
    )
    .await?;
    info!(
        remote_tls = %addr,
        remote_node = %welcome.node,
        protocol = %peer_transport::PeerProtocol::TlsTcp,
        "connected to direct TLS peer transport"
    );
    Ok(opened_remote(stream, peer_transport::PeerProtocol::TlsTcp))
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

async fn open_exec_helper(
    args: &cli::ProxyArgs,
    client: &ssh_client::Client,
) -> Result<OpenedRemoteHelper> {
    let remote_path = ensure_helper(args, client, HelperCapability::Stdio).await?;
    let remote_os = match args.remote_os {
        cli::RemoteOs::Auto => cli::RemoteOs::Unix,
        other => other,
    };
    let command = remote_stdio_command(&remote_path, remote_os);
    info!(host = %client.target().host, user = %client.target().user, %remote_path, "starting remote helper through russh exec");
    Ok(opened_remote(
        client.exec_stream(command).await?,
        peer_transport::PeerProtocol::SshExec,
    ))
}

fn opened_remote<S>(stream: S, protocol: peer_transport::PeerProtocol) -> OpenedRemoteHelper
where
    S: RemoteStream + 'static,
{
    OpenedRemoteHelper {
        stream: Box::new(stream),
        protocol,
        timings: RemoteHelperTimings::default(),
    }
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn local_node_name() -> String {
    format!(
        "{}@{}",
        whoami::username().unwrap_or_else(|_| "unknown".to_string()),
        std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "unknown".to_string())
    )
}

pub async fn open_remote_reverse_socks(
    args: &cli::ProxyArgs,
    remote_listen: SocketAddr,
) -> Result<ssh_client::SshStream> {
    let client = ssh_client::Client::connect_proxy_args(args).await?;
    let remote_path = ensure_helper(
        args,
        &client,
        HelperCapability::ReverseSocks {
            listen: remote_listen,
        },
    )
    .await?;
    let remote_os = match args.remote_os {
        cli::RemoteOs::Auto => cli::RemoteOs::Unix,
        other => other,
    };
    let command = remote_reverse_socks_command(&remote_path, remote_os, remote_listen);
    info!(host = %client.target().host, user = %client.target().user, %remote_path, %remote_listen, "starting reverse SOCKS helper through russh exec");
    client.exec_stream(command).await
}
