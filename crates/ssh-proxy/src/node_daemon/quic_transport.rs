use std::{
    net::SocketAddr,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use tokio::{io::AsyncWriteExt, sync::watch, time};
use tracing::{debug, info, trace, warn};

use crate::{peer_transport, quic_native, quic_stream, remote};

use super::NodeManager;

const SUPPORTED_QUIC_PROTOCOLS: &[peer_transport::PeerProtocol] = &[
    peer_transport::PeerProtocol::QuicNative,
    peer_transport::PeerProtocol::Quic,
];

enum NativeQuicStreamOutcome {
    Completed((u64, u64)),
    FirstByteTimeout,
    CopyFailed(anyhow::Error),
}

pub(super) async fn handle_connection(
    connection: quinn::Connection,
    peer: SocketAddr,
    manager: Arc<NodeManager>,
) {
    let result = handle_connection_inner(connection.clone(), peer, manager).await;
    if let Err(err) = result {
        warn!(%peer, error = %err, "node QUIC transport failed");
        connection.close(0_u32.into(), b"error");
    }
}

async fn handle_connection_inner(
    connection: quinn::Connection,
    peer: SocketAddr,
    manager: Arc<NodeManager>,
) -> Result<()> {
    let (send, recv) = connection
        .accept_bi()
        .await
        .context("node QUIC transport stream accept failed")?;
    let mut stream = quic_stream::QuicBiStream::new(send, recv);
    let hello = peer_transport::server_handshake(
        &mut stream,
        manager.name.clone(),
        SUPPORTED_QUIC_PROTOCOLS,
    )
    .await
    .context("node QUIC transport handshake failed")?;
    let accepted =
        peer_transport::select_supported_protocol(&hello.protocols, SUPPORTED_QUIC_PROTOCOLS)
            .ok_or_else(|| anyhow::anyhow!("accepted QUIC protocol is missing after handshake"))?;
    info!(
        %peer,
        remote_node = %hello.node,
        protocols = ?hello.protocols,
        accepted = %accepted,
        data_plane = accepted.data_plane_label(),
        "node QUIC transport handshake completed"
    );

    match accepted {
        peer_transport::PeerProtocol::Quic => run_framed_quic(stream, peer, manager).await,
        peer_transport::PeerProtocol::QuicNative => {
            run_native_quic(connection, stream, peer, manager).await
        }
        other => bail!("unexpected QUIC transport protocol {other}"),
    }?;
    Ok(())
}

async fn run_framed_quic(
    stream: quic_stream::QuicBiStream,
    peer: SocketAddr,
    manager: Arc<NodeManager>,
) -> Result<()> {
    manager.total_transports.fetch_add(1, Ordering::Relaxed);
    manager.active_transports.fetch_add(1, Ordering::Relaxed);
    let (reader, writer) = tokio::io::split(stream);
    let result = remote::run_transport(reader, writer).await;
    manager
        .active_transports
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(1))
        .ok();
    result.with_context(|| format!("node QUIC framed transport failed for {peer}"))
}

async fn run_native_quic(
    connection: quinn::Connection,
    mut control_stream: quic_stream::QuicBiStream,
    peer: SocketAddr,
    manager: Arc<NodeManager>,
) -> Result<()> {
    let hello = quic_native::session::server_accept(&mut control_stream, |hello| {
        quic_native::RouteControlWelcome {
            version: quic_native::CONTROL_FRAME_VERSION,
            route_id: hello.route_id.clone(),
            accepted: true,
            selected_protocol: Some(peer_transport::PeerProtocol::QuicNative.to_string()),
            message: "quic-native control session ready".to_string(),
        }
    })
    .await
    .context("node QUIC-native route session failed")?;
    info!(
        %peer,
        route_id = %hello.route_id,
        remote_node = %hello.node,
        "node QUIC-native route session established"
    );
    let route_id = hello.route_id.clone();

    let control_task = tokio::spawn(async move {
        if let Err(err) = run_native_quic_control_loop(control_stream).await {
            warn!(%peer, error = %err, "node QUIC-native control loop stopped");
        }
    });
    let close_connection = connection.clone();
    let result = run_native_quic_data_plane(connection, peer, manager, route_id).await;
    control_task.abort();
    close_connection.close(0_u32.into(), b"done");
    result
}

async fn run_native_quic_control_loop(mut stream: quic_stream::QuicBiStream) -> Result<()> {
    loop {
        let frame = quic_native::control::RouteControlFrame::read_from(&mut stream)
            .await
            .context("node QUIC-native control frame read failed")?;
        match frame {
            quic_native::control::RouteControlFrame::Ping { seq } => {
                debug!(seq, "node QUIC-native control ping received");
                quic_native::control::RouteControlFrame::Pong { seq }
                    .write_to(&mut stream)
                    .await
                    .context("node QUIC-native control pong write failed")?;
                debug!(seq, "node QUIC-native control pong sent");
            }
            quic_native::control::RouteControlFrame::Pong { seq } => {
                debug!(seq, "node QUIC-native control pong received");
            }
            quic_native::control::RouteControlFrame::Hello(_)
            | quic_native::control::RouteControlFrame::Welcome(_) => {}
        }
    }
}

async fn run_native_quic_data_plane(
    connection: quinn::Connection,
    peer: SocketAddr,
    manager: Arc<NodeManager>,
    route_id: String,
) -> Result<()> {
    manager.total_transports.fetch_add(1, Ordering::Relaxed);
    manager.active_transports.fetch_add(1, Ordering::Relaxed);
    let result = async {
        loop {
            let (send, recv) = match connection.accept_bi().await {
                Ok(streams) => streams,
                Err(err) => {
                    warn!(%peer, error = %err, "node QUIC-native stream accept ended");
                    break;
                }
            };
            trace!(%peer, route_id = %route_id, "accepted QUIC-native bidi stream");
            let peer = peer;
            let route_id = route_id.clone();
            tokio::spawn(async move {
                if let Err(err) = handle_native_quic_stream(send, recv, peer, route_id).await {
                    warn!(%peer, error = %err, "node QUIC-native stream failed");
                }
            });
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    manager
        .active_transports
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(1))
        .ok();
    result.with_context(|| format!("node QUIC-native transport failed for {peer}"))
}

async fn handle_native_quic_stream(
    send: quinn::SendStream,
    recv: quinn::RecvStream,
    peer: SocketAddr,
    route_id: String,
) -> Result<()> {
    let mut stream = quic_stream::QuicBiStream::new(send, recv);
    let header_started = std::time::Instant::now();
    let header = time::timeout(
        Duration::from_secs(10),
        quic_native::flow::read_flow_header(&mut stream),
    )
    .await
    .context("node QUIC-native flow header timed out")??;
    debug!(
        %peer,
        route_id = %header.route_id,
        stream_id = header.stream_id,
        header_read_latency_ms = header_started.elapsed().as_millis(),
        "node QUIC-native flow header read"
    );
    if header.route_id != route_id {
        debug!(
            %peer,
            expected_route_id = %route_id,
            actual_route_id = %header.route_id,
            stream_id = header.stream_id,
            "node QUIC-native stream reset after route mismatch"
        );
        stream.reset(quinn::VarInt::from_u32(quic_native::FLOW_RESET_ERROR_CODE));
        bail!(
            "node QUIC-native route mismatch: expected {}, got {}",
            route_id,
            header.route_id
        );
    }
    let mut remote = match time::timeout(
        Duration::from_secs(10),
        remote::egress::connect_tcp(
            &header.target.host,
            header.target.port,
            header.egress_proxy.as_deref(),
        ),
    )
    .await
    .context("node QUIC-native egress open timed out")?
    .with_context(|| {
        format!(
            "failed to open egress tcp {}:{} for node QUIC-native stream",
            header.target.host, header.target.port
        )
    }) {
        Ok(remote) => remote,
        Err(err) => {
            debug!(
                %peer,
                route_id = %header.route_id,
                stream_id = header.stream_id,
                error = %err,
                "node QUIC-native stream reset after egress open failure"
            );
            stream.reset(quinn::VarInt::from_u32(quic_native::FLOW_RESET_ERROR_CODE));
            return Err(err);
        }
    };
    info!(
        %peer,
        route_id = %header.route_id,
        stream_id = header.stream_id,
        target = %format!("{}:{}", header.target.host, header.target.port),
        "node QUIC-native stream bridged"
    );
    let stream_id = header.stream_id;
    let route_id_for_logs = header.route_id.clone();
    let first_byte_recorded = stream.first_byte_recorded();
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let watchdog = tokio::spawn({
        let first_byte_recorded = first_byte_recorded.clone();
        let route_id_for_logs = route_id_for_logs.clone();
        async move {
            time::sleep(quic_native::QUIC_NATIVE_FIRST_BYTE_TIMEOUT).await;
            if !first_byte_recorded.load(Ordering::Relaxed) {
                let _ = cancel_tx.send(true);
                debug!(
                    %peer,
                    route_id = %route_id_for_logs,
                    stream_id = stream_id,
                    timeout_secs = quic_native::QUIC_NATIVE_FIRST_BYTE_TIMEOUT.as_secs(),
                    "node QUIC-native first byte timeout fired"
                );
            }
        }
    });
    let mut cancel_rx = cancel_rx;
    let outcome = tokio::select! {
        result = tokio::io::copy_bidirectional_with_sizes(
            &mut stream,
            &mut remote,
            quic_native::QUIC_NATIVE_COPY_BUFFER_SIZE,
            quic_native::QUIC_NATIVE_COPY_BUFFER_SIZE,
        ) => match result {
            Ok(counts) => NativeQuicStreamOutcome::Completed(counts),
            Err(err) => NativeQuicStreamOutcome::CopyFailed(err.into()),
        },
        _ = cancel_rx.changed() => {
            stream.reset(quinn::VarInt::from_u32(quic_native::FLOW_RESET_ERROR_CODE));
            remote.shutdown().await.ok();
            NativeQuicStreamOutcome::FirstByteTimeout
        }
    };
    watchdog.abort();
    match outcome {
        NativeQuicStreamOutcome::Completed((client_to_remote, remote_to_client)) => {
            info!(
                %peer,
                route_id = %route_id_for_logs,
                stream_id = stream_id,
                client_to_remote,
                remote_to_client,
                "node QUIC-native stream completed"
            );
            stream.finish();
            let _ = remote.shutdown().await;
            Ok(())
        }
        NativeQuicStreamOutcome::FirstByteTimeout => Err(anyhow::anyhow!(
            "node QUIC-native first byte timed out after {}s",
            quic_native::QUIC_NATIVE_FIRST_BYTE_TIMEOUT.as_secs()
        )),
        NativeQuicStreamOutcome::CopyFailed(err) => {
            debug!(
                %peer,
                route_id = %route_id_for_logs,
                stream_id = stream_id,
                error = %err,
                "node QUIC-native stream reset after copy failure"
            );
            stream.reset(quinn::VarInt::from_u32(quic_native::FLOW_RESET_ERROR_CODE));
            Err(err).context("node QUIC-native stream copy failed")
        }
    }
}
