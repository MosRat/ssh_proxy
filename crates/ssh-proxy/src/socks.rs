use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Instant,
};

use anyhow::{Result, bail};
use ssh_proxy_transport::proxy::{
    http::write_http_error,
    socks5::{
        Command, Reply, Request, SOCKS_VERSION, build_udp_packet, negotiate_no_auth,
        parse_udp_packet, reply,
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    sync::Mutex,
};
use tracing::warn;

use crate::{cli, controller, data_plane, protocol::UdpDatagram, quic_native, ssh_native};

mod relay;
mod tunnel;

use tunnel::{TunnelBackend, TunnelResponse};

pub async fn handle_client(
    mut stream: TcpStream,
    peer: SocketAddr,
    state: Arc<controller::SharedState>,
) -> Result<()> {
    let _ = stream.set_nodelay(true);
    let mut first = [0_u8; 1];
    stream.peek(&mut first).await?;
    if first[0] != SOCKS_VERSION {
        return tunnel::handle_http_proxy(stream, peer, TunnelBackend::spx(state)).await;
    }
    negotiate_no_auth(&mut stream).await?;
    let request = Request::read_from(&mut stream).await?;
    match request.command {
        Command::Connect => handle_connect(stream, peer, request, TunnelBackend::spx(state)).await,
        Command::UdpAssociate => handle_udp_associate(stream, peer, state).await,
        Command::Bind => {
            reply(
                &mut stream,
                Reply::CommandNotSupported,
                SocketAddr::from(([0, 0, 0, 0], 0)),
            )
            .await?;
            bail!("SOCKS BIND is not supported")
        }
    }
}

pub async fn handle_fixed_target(
    stream: TcpStream,
    peer: SocketAddr,
    target: cli::TcpTarget,
    state: Arc<controller::SharedState>,
) -> Result<()> {
    let _ = stream.set_nodelay(true);
    tunnel::open_tunnel(
        stream,
        peer,
        target.host,
        target.port,
        TunnelBackend::spx(state),
        TunnelResponse::None,
        Vec::new(),
    )
    .await
}

pub async fn handle_client_ssh_native(
    mut stream: TcpStream,
    peer: SocketAddr,
    state: Arc<ssh_native::State>,
) -> Result<()> {
    let _ = stream.set_nodelay(true);
    let mut first = [0_u8; 1];
    stream.peek(&mut first).await?;
    if first[0] != SOCKS_VERSION {
        return tunnel::handle_http_proxy(stream, peer, TunnelBackend::ssh_native(state)).await;
    }
    negotiate_no_auth(&mut stream).await?;
    let request = Request::read_from(&mut stream).await?;
    match request.command {
        Command::Connect => {
            tunnel::open_tunnel(
                stream,
                peer,
                request.host,
                request.port,
                TunnelBackend::ssh_native(state),
                TunnelResponse::SocksSuccess,
                Vec::new(),
            )
            .await
        }
        Command::UdpAssociate | Command::Bind => {
            reply(
                &mut stream,
                Reply::CommandNotSupported,
                SocketAddr::from(([0, 0, 0, 0], 0)),
            )
            .await?;
            bail!("ssh-native currently supports only TCP CONNECT")
        }
    }
}

pub async fn handle_fixed_target_ssh_native(
    stream: TcpStream,
    peer: SocketAddr,
    target: cli::TcpTarget,
    state: Arc<ssh_native::State>,
) -> Result<()> {
    let _ = stream.set_nodelay(true);
    tunnel::open_tunnel(
        stream,
        peer,
        target.host,
        target.port,
        TunnelBackend::ssh_native(state),
        TunnelResponse::None,
        Vec::new(),
    )
    .await
}

pub async fn handle_client_quic_native(
    mut stream: TcpStream,
    peer: SocketAddr,
    state: Arc<quic_native::State>,
) -> Result<()> {
    let _ = stream.set_nodelay(true);
    let mut first = [0_u8; 1];
    stream.peek(&mut first).await?;
    if first[0] != SOCKS_VERSION {
        return tunnel::handle_http_proxy(
            stream,
            peer,
            TunnelBackend::quic_native(state, quic_native::TargetKind::TcpConnect),
        )
        .await;
    }
    negotiate_no_auth(&mut stream).await?;
    let request = Request::read_from(&mut stream).await?;
    match request.command {
        Command::Connect => {
            tunnel::open_tunnel(
                stream,
                peer,
                request.host,
                request.port,
                TunnelBackend::quic_native(state, quic_native::TargetKind::TcpConnect),
                TunnelResponse::SocksSuccess,
                Vec::new(),
            )
            .await
        }
        Command::UdpAssociate | Command::Bind => {
            reply(
                &mut stream,
                Reply::CommandNotSupported,
                SocketAddr::from(([0, 0, 0, 0], 0)),
            )
            .await?;
            bail!("quic-native currently supports only TCP CONNECT")
        }
    }
}

pub async fn handle_fixed_target_quic_native(
    stream: TcpStream,
    peer: SocketAddr,
    target: cli::TcpTarget,
    state: Arc<quic_native::State>,
) -> Result<()> {
    let _ = stream.set_nodelay(true);
    tunnel::open_tunnel(
        stream,
        peer,
        target.host,
        target.port,
        TunnelBackend::quic_native(state, quic_native::TargetKind::FixedTcp),
        TunnelResponse::None,
        Vec::new(),
    )
    .await
}

async fn handle_connect(
    stream: TcpStream,
    peer: SocketAddr,
    request: Request,
    backend: TunnelBackend,
) -> Result<()> {
    let host = request.host;
    let port = request.port;
    tunnel::open_tunnel(
        stream,
        peer,
        host,
        port,
        backend,
        TunnelResponse::SocksSuccess,
        Vec::new(),
    )
    .await
}

async fn open_tunnel(
    mut stream: TcpStream,
    peer: SocketAddr,
    host: String,
    port: u16,
    state: Arc<controller::SharedState>,
    response: TunnelResponse,
    initial_remote: Vec<u8>,
) -> Result<()> {
    state.record_tcp_open();
    let _guard = TcpConnGuard {
        state: state.clone(),
    };
    state.record_tcp_open_attempt();
    let open_started = Instant::now();
    let bridge = match state.bridge().await {
        Ok(bridge) => bridge,
        Err(err) => {
            state.record_tcp_open_latency(open_started.elapsed());
            state.record_tcp_open_failure(err.to_string()).await;
            write_tunnel_open_error(&mut stream, &response).await?;
            return Err(err);
        }
    };
    let worker_slot = bridge.slot;
    let link = data_plane::SpxRouteLink::new(bridge.handle);
    let target = data_plane::TcpTarget::new(host.clone(), port, state.egress_proxy());
    let remote_flow = match link.open_tcp(target).await {
        Ok(opened) => {
            state.record_tcp_open_latency(open_started.elapsed());
            state.record_tcp_open_success().await;
            state.record_worker_tcp_open(worker_slot);
            opened
        }
        Err(err) => {
            state.record_tcp_open_latency(open_started.elapsed());
            state.record_tcp_open_failure(err.to_string()).await;
            warn!(
                %peer,
                host = %host,
                port = port,
                error = %err,
                "proxy CONNECT failed through bridge"
            );
            write_tunnel_open_error(&mut stream, &response).await?;
            return Err(err);
        }
    };
    match response {
        TunnelResponse::SocksSuccess => {
            reply(
                &mut stream,
                Reply::Succeeded,
                SocketAddr::from(([0, 0, 0, 0], 0)),
            )
            .await?;
        }
        TunnelResponse::HttpConnect => {
            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await?;
        }
        TunnelResponse::None => {}
    }
    let result = relay::relay_spx_tcp(
        stream,
        initial_remote,
        remote_flow,
        state.clone(),
        worker_slot,
    )
    .await;
    state.record_worker_tcp_close(worker_slot);
    result
}

async fn open_tunnel_ssh_native(
    mut stream: TcpStream,
    peer: SocketAddr,
    host: String,
    port: u16,
    state: Arc<ssh_native::State>,
    response: TunnelResponse,
    initial_remote: Vec<u8>,
) -> Result<()> {
    state.record_tcp_open();
    let _guard = SshNativeTcpConnGuard {
        state: state.clone(),
    };
    state.record_tcp_open_attempt();
    let open_started = Instant::now();
    let remote = match state.open_stream(host.clone(), port).await {
        Ok(remote) => {
            state.record_tcp_open_latency(open_started.elapsed());
            state.record_tcp_open_success().await;
            remote
        }
        Err(err) => {
            state.record_tcp_open_latency(open_started.elapsed());
            state.record_tcp_open_failure(err.to_string()).await;
            warn!(
                %peer,
                host = %host,
                port = port,
                error = %err,
                "ssh-native CONNECT failed"
            );
            write_tunnel_open_error(&mut stream, &response).await?;
            return Err(err);
        }
    };

    match response {
        TunnelResponse::SocksSuccess => {
            reply(
                &mut stream,
                Reply::Succeeded,
                SocketAddr::from(([0, 0, 0, 0], 0)),
            )
            .await?;
        }
        TunnelResponse::HttpConnect => {
            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await?;
        }
        TunnelResponse::None => {}
    }

    relay::relay_ssh_native_tcp(stream, remote, initial_remote, state).await
}

async fn open_tunnel_quic_native(
    mut stream: TcpStream,
    peer: SocketAddr,
    host: String,
    port: u16,
    kind: quic_native::TargetKind,
    state: Arc<quic_native::State>,
    response: TunnelResponse,
    initial_remote: Vec<u8>,
) -> Result<()> {
    state.record_tcp_open();
    let _guard = QuicNativeTcpConnGuard {
        state: state.clone(),
    };
    state.record_tcp_open_attempt();
    let open_started = Instant::now();
    let target = quic_native::StreamTarget {
        kind,
        host: host.clone(),
        port,
    };
    let egress_proxy = state.egress_proxy();
    let remote = match state.open_stream(target, egress_proxy).await {
        Ok(remote) => {
            state.record_tcp_open_latency(open_started.elapsed());
            state.record_tcp_open_success().await;
            remote
        }
        Err(err) => {
            state.record_tcp_open_latency(open_started.elapsed());
            state.record_tcp_open_failure(err.to_string()).await;
            warn!(
                %peer,
                host = %host,
                port = port,
                error = %err,
                "quic-native CONNECT failed"
            );
            write_tunnel_open_error(&mut stream, &response).await?;
            return Err(err);
        }
    };

    match response {
        TunnelResponse::SocksSuccess => {
            reply(
                &mut stream,
                Reply::Succeeded,
                SocketAddr::from(([0, 0, 0, 0], 0)),
            )
            .await?;
        }
        TunnelResponse::HttpConnect => {
            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await?;
        }
        TunnelResponse::None => {}
    }

    relay::relay_quic_native_tcp(stream, remote, initial_remote, state).await
}

async fn write_tunnel_open_error(stream: &mut TcpStream, response: &TunnelResponse) -> Result<()> {
    match response {
        TunnelResponse::SocksSuccess => {
            reply(
                stream,
                Reply::HostUnreachable,
                SocketAddr::from(([0, 0, 0, 0], 0)),
            )
            .await?;
        }
        TunnelResponse::HttpConnect | TunnelResponse::None => {
            write_http_error(stream, 502, "Bad Gateway").await?;
        }
    }
    Ok(())
}

struct TcpConnGuard {
    state: Arc<controller::SharedState>,
}

impl Drop for TcpConnGuard {
    fn drop(&mut self) {
        self.state.record_tcp_close();
    }
}

struct SshNativeTcpConnGuard {
    state: Arc<ssh_native::State>,
}

impl Drop for SshNativeTcpConnGuard {
    fn drop(&mut self) {
        self.state.record_tcp_close();
    }
}

struct QuicNativeTcpConnGuard {
    state: Arc<quic_native::State>,
}

impl Drop for QuicNativeTcpConnGuard {
    fn drop(&mut self) {
        self.state.record_tcp_close();
    }
}

async fn handle_udp_associate(
    mut control: TcpStream,
    peer: SocketAddr,
    state: Arc<controller::SharedState>,
) -> Result<()> {
    let bridge = state.bridge().await?;
    let bind_ip = match control.local_addr()?.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() => IpAddr::V4(ip),
        IpAddr::V6(ip) if !ip.is_unspecified() => IpAddr::V6(ip),
        _ => IpAddr::V4(Ipv4Addr::LOCALHOST),
    };
    let udp = Arc::new(UdpSocket::bind(SocketAddr::new(bind_ip, 0)).await?);
    let udp_addr = udp.local_addr()?;
    reply(&mut control, Reply::Succeeded, udp_addr).await?;

    let link = data_plane::SpxRouteLink::new(bridge.handle);
    let udp_flow = link.register_udp().await;
    let (udp_tx, mut remote_rx, udp_close) = udp_flow.split();
    let client_addr = Arc::new(Mutex::new(Some(peer)));
    let udp_to_remote = {
        let udp = udp.clone();
        let client_addr = client_addr.clone();
        tokio::spawn(async move {
            let mut buf = vec![0_u8; 65535];
            loop {
                let (n, from) = udp.recv_from(&mut buf).await?;
                *client_addr.lock().await = Some(from);
                let packet = parse_udp_packet(&buf[..n])?;
                udp_tx
                    .send(UdpDatagram {
                        host: packet.host,
                        port: packet.port,
                        data: packet.data,
                    })
                    .await?;
            }
            #[allow(unreachable_code)]
            Ok::<_, anyhow::Error>(())
        })
    };

    let remote_to_udp = {
        let udp = udp.clone();
        let client_addr = client_addr.clone();
        tokio::spawn(async move {
            while let Some(packet) = remote_rx.recv().await {
                let response = build_udp_packet(&packet.host, packet.port, &packet.data)?;
                let addr = *client_addr.lock().await;
                if let Some(addr) = addr {
                    udp.send_to(&response, addr).await?;
                }
            }
            Ok::<_, anyhow::Error>(())
        })
    };

    let mut one = [0_u8; 1];
    let _ = control.read(&mut one).await;
    udp_to_remote.abort();
    remote_to_udp.abort();
    udp_close.close().await;
    Ok(())
}
