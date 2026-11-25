use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use bytes::{Bytes, BytesMut};
use tokio::{
    io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::{RwLock, mpsc},
    time,
};
use tracing::{debug, info, warn};

use crate::{
    bridge, cli, controller, peer_transport,
    protocol::{
        FRAME_CHANNEL_CAPACITY, Frame, FrameReader, TCP_DATA_CHUNK, TCP_STREAM_CHANNEL_CAPACITY,
        UdpDatagram, write_frame_batch,
    },
    socks,
};

mod admin;
pub(crate) mod egress;

const TCP_AUTH_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn run(args: cli::RemoteArgs) -> Result<()> {
    if let Some(command) = args.command {
        match command {
            cli::RemoteCommand::Admin { json } => return admin::run(json).await,
        }
    }
    if let Some(addr) = args.reverse_socks {
        run_reverse_socks(addr, args.tcp_target).await
    } else if let Some(addr) = args.tcp_listen {
        run_tcp(addr, args.token).await
    } else {
        run_stdio().await
    }
}

async fn run_stdio() -> Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    run_transport(stdin, stdout).await
}

async fn run_reverse_socks(addr: SocketAddr, target: Option<cli::TcpTarget>) -> Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let bridge = bridge::connect_io(stdin, stdout).await?;
    let state = Arc::new(controller::SharedState::new_with_bridge(
        bridge.handle.clone(),
    ));
    let mut lifecycle = bridge.lifecycle;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind remote reverse SOCKS listener {addr}"))?;
    info!(%addr, "remote reverse listener ready");
    bridge
        .handle
        .send_log(format!("remote reverse listener ready at {addr}"))
        .await;

    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, peer) = accept?;
                let _ = stream.set_nodelay(true);
                let state = state.clone();
                let target = target.clone();
                tokio::spawn(async move {
                    let result = match target {
                        Some(target) => socks::handle_fixed_target(stream, peer, target, state).await,
                        None => socks::handle_client(stream, peer, state).await,
                    };
                    if let Err(err) = result {
                        debug!(%peer, error = %err, "remote reverse SOCKS client failed");
                    }
                });
            }
            _ = &mut lifecycle => break,
        }
    }
    Ok(())
}

async fn run_tcp(addr: SocketAddr, token: Option<String>) -> Result<()> {
    let listener = TcpListener::bind(addr).await?;
    info!(%addr, "remote framed TCP transport listening");
    loop {
        let (mut stream, peer) = listener.accept().await?;
        let _ = stream.set_nodelay(true);
        let token = token.clone();
        tokio::spawn(async move {
            if let Err(err) = verify_token(&mut stream, token.as_deref()).await {
                warn!(%peer, error = %err, "remote TCP auth failed");
                return;
            }
            let hello = match peer_transport::server_handshake(
                &mut stream,
                "remote-helper",
                &[
                    peer_transport::PeerProtocol::SshDirect,
                    peer_transport::PeerProtocol::Tcp,
                ],
            )
            .await
            {
                Ok(hello) => hello,
                Err(err) => {
                    warn!(%peer, error = %err, "remote TCP handshake failed");
                    return;
                }
            };
            info!(
                %peer,
                remote_node = %hello.node,
                protocols = ?hello.protocols,
                "remote TCP handshake completed"
            );
            let (reader, writer) = io::split(stream);
            if let Err(err) = run_transport(reader, writer).await {
                warn!(%peer, error = %err, "remote TCP transport failed");
            }
        });
    }
}

async fn verify_token(stream: &mut TcpStream, expected: Option<&str>) -> Result<()> {
    if let Some(expected) = expected {
        time::timeout(TCP_AUTH_TIMEOUT, async {
            let mut len = [0_u8; 2];
            stream.read_exact(&mut len).await?;
            let len = u16::from_be_bytes(len) as usize;
            let mut token = vec![0_u8; len];
            stream.read_exact(&mut token).await?;
            if token != expected.as_bytes() {
                bail!("invalid token");
            }
            Ok(())
        })
        .await
        .with_context(|| {
            format!(
                "remote TCP auth timed out after {}s",
                TCP_AUTH_TIMEOUT.as_secs()
            )
        })??;
    }
    Ok(())
}

pub async fn run_transport<R, W>(reader: R, writer: W) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    run_transport_with_egress_proxy(reader, writer, None).await
}

pub async fn run_transport_with_egress_proxy<R, W>(
    mut reader: R,
    writer: W,
    default_egress_proxy: Option<String>,
) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut rx) = mpsc::channel::<Frame>(FRAME_CHANNEL_CAPACITY);
    let tcp_streams = Arc::new(RwLock::new(HashMap::<u32, mpsc::Sender<Bytes>>::new()));
    let udp_streams = Arc::new(RwLock::new(HashMap::<u32, mpsc::Sender<UdpDatagram>>::new()));

    let write_frames = async move {
        let mut writer = writer;
        while let Some(frame) = rx.recv().await {
            write_frame_batch(&mut writer, frame, &mut rx).await?;
        }
        Ok(())
    };
    tokio::pin!(write_frames);
    let mut flows = tokio::task::JoinSet::new();
    let mut frame_reader = FrameReader::new();
    loop {
        let frame = tokio::select! {
            result = &mut write_frames => return result,
            frame = frame_reader.read_from(&mut reader) => frame?,
        };
        let Some(frame) = frame else { break };
        while flows.try_join_next().is_some() {}
        match frame {
            Frame::OpenTcp {
                id,
                host,
                port,
                egress_proxy,
            } => {
                let tx = tx.clone();
                let tcp_streams = tcp_streams.clone();
                let egress_proxy = egress_proxy.or_else(|| default_egress_proxy.clone());
                flows.spawn(async move {
                    handle_open_tcp(id, host, port, egress_proxy, tx, tcp_streams).await;
                });
            }
            Frame::Data { id, data } => {
                let stream_tx = tcp_streams.read().await.get(&id).cloned();
                if let Some(stream_tx) = stream_tx {
                    match stream_tx.try_send(data) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            tcp_streams.write().await.remove(&id);
                        }
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            tcp_streams.write().await.remove(&id);
                            tx.try_send(Frame::Close {
                                id,
                                reason: "remote TCP receiver queue full".into(),
                            })
                            .context("cannot signal overloaded remote TCP stream")?;
                        }
                    }
                }
            }
            Frame::Close { id, .. } => {
                tcp_streams.write().await.remove(&id);
                udp_streams.write().await.remove(&id);
            }
            Frame::UdpPacket {
                id,
                host,
                port,
                data,
            } => {
                let stream_tx = {
                    let mut streams = udp_streams.write().await;
                    if let Some(stream_tx) = streams.get(&id).cloned() {
                        stream_tx
                    } else {
                        let (stream_tx, stream_rx) = mpsc::channel(TCP_STREAM_CHANNEL_CAPACITY);
                        streams.insert(id, stream_tx.clone());
                        flows.spawn(run_udp_assoc(id, tx.clone(), stream_rx));
                        stream_tx
                    }
                };
                if let Err(mpsc::error::TrySendError::Closed(_)) =
                    stream_tx.try_send(UdpDatagram { host, port, data })
                {
                    udp_streams.write().await.remove(&id);
                }
            }
            Frame::Log { message } => info!(target: "remote", %message),
            Frame::OpenTcpResult { .. } => {}
        }
    }

    flows.shutdown().await;
    Ok(())
}

async fn handle_open_tcp(
    id: u32,
    host: String,
    port: u16,
    egress_proxy: Option<String>,
    tx: mpsc::Sender<Frame>,
    tcp_streams: Arc<RwLock<HashMap<u32, mpsc::Sender<Bytes>>>>,
) {
    match egress::connect_tcp(&host, port, egress_proxy.as_deref()).await {
        Ok(stream) => {
            let (stream_tx, mut stream_rx) = mpsc::channel::<Bytes>(TCP_STREAM_CHANNEL_CAPACITY);
            tcp_streams.write().await.insert(id, stream_tx);
            let _ = tx
                .send(Frame::OpenTcpResult {
                    id,
                    ok: true,
                    message: String::new(),
                })
                .await;
            let (mut reader, mut writer) = stream.into_split();

            let tx_reader = tx.clone();
            let remote_to_local = async move {
                let mut buf = BytesMut::with_capacity(TCP_DATA_CHUNK);
                loop {
                    if buf.capacity() < TCP_DATA_CHUNK {
                        buf.reserve(TCP_DATA_CHUNK - buf.capacity());
                    }
                    let n = reader.read_buf(&mut buf).await?;
                    if n == 0 {
                        break;
                    }
                    tx_reader
                        .send(Frame::Data {
                            id,
                            data: buf.split().freeze(),
                        })
                        .await
                        .ok();
                }
                Ok::<_, anyhow::Error>(())
            };

            let local_to_remote = async move {
                while let Some(data) = stream_rx.recv().await {
                    writer.write_all(&data).await?;
                }
                writer.shutdown().await?;
                Ok::<_, anyhow::Error>(())
            };

            tokio::select! {
                _ = remote_to_local => {}
                _ = local_to_remote => {}
            }
            tcp_streams.write().await.remove(&id);
            let _ = tx
                .send(Frame::Close {
                    id,
                    reason: "tcp closed".to_string(),
                })
                .await;
        }
        Err(err) => {
            warn!(id, %host, port, error = %err, "egress TCP connect failed");
            let _ = tx
                .send(Frame::OpenTcpResult {
                    id,
                    ok: false,
                    message: err.to_string(),
                })
                .await;
        }
    }
}

async fn run_udp_assoc(id: u32, tx: mpsc::Sender<Frame>, mut rx: mpsc::Receiver<UdpDatagram>) {
    let socket = match UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).await {
        Ok(socket) => socket,
        Err(err) => {
            let _ = tx
                .send(Frame::Close {
                    id,
                    reason: format!("udp bind failed: {err}"),
                })
                .await;
            return;
        }
    };

    let socket = Arc::new(socket);
    let reader_socket = socket.clone();
    let reader_tx = tx.clone();
    let reader = async move {
        let mut buf = vec![0_u8; 65535];
        loop {
            let (n, from) = reader_socket.recv_from(&mut buf).await?;
            reader_tx
                .send(Frame::UdpPacket {
                    id,
                    host: from.ip().to_string(),
                    port: from.port(),
                    data: buf[..n].to_vec(),
                })
                .await
                .ok();
        }
        #[allow(unreachable_code)]
        Ok::<_, anyhow::Error>(())
    };

    let outgoing = async {
        while let Some(packet) = rx.recv().await {
            if let Err(err) = socket
                .send_to(&packet.data, (packet.host.as_str(), packet.port))
                .await
            {
                warn!(error = %err, "remote UDP send failed");
            }
        }
    };
    tokio::select! { _ = reader => {}, _ = outgoing => {} }
}
