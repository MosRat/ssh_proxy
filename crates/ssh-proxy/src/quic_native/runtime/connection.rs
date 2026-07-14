use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow};
use ssh_proxy_transport::quic::connect_client;
use tracing::debug;

use crate::{
    cli, peer_transport,
    quic_native::{
        metrics::duration_millis,
        runtime_config::{local_node_name, quic_options_from_proxy_args},
        session::{RouteSessionSpec, client_negotiate},
    },
    quic_stream,
};

use super::ConnectionWorker;

pub(super) struct ConnectedWorker {
    pub(super) worker: Arc<ConnectionWorker>,
    pub(super) control: quic_stream::QuicBiStream,
    pub(super) route_id: String,
    pub(super) _session_node: String,
}

pub(super) async fn connect_worker(
    args: &cli::ProxyArgs,
    route_id: &str,
    worker_id: usize,
) -> Result<ConnectedWorker> {
    let addr = args
        .remote_quic
        .ok_or_else(|| anyhow!("--remote-quic is required for quic-native transport"))?;
    let ca = args
        .remote_ca
        .as_deref()
        .ok_or_else(|| anyhow!("--remote-ca is required for quic-native transport"))?;
    let roots = peer_transport::load_cert_chain(ca)?;
    let quic_options = quic_options_from_proxy_args(args)?;
    debug!(
        worker_id,
        remote_quic = %addr,
        remote_name = %args.remote_name,
        quic_udp_runtime = peer_transport::QUIC_UDP_RUNTIME,
        quic_udp_gso_source = peer_transport::QUIC_UDP_GSO_SOURCE,
        quic_packetization = peer_transport::QUIC_PACKETIZATION,
        ?quic_options,
        "connecting QUIC-native worker"
    );
    let connection = connect_client(
        addr,
        &args.remote_name,
        roots,
        quic_options,
        Duration::from_secs(args.connect_timeout_secs.max(1)),
        format!("connect remote QUIC-native transport {addr}"),
    )
    .await
    .with_context(|| format!("failed to connect remote QUIC-native transport {addr}"))?;
    let control_open_started = std::time::Instant::now();
    let mut control = connection
        .open_bi(
            Duration::from_secs(args.connect_timeout_secs.max(1)),
            "open remote QUIC-native control stream",
        )
        .await
        .context("failed to open QUIC-native control stream")?;
    debug!(
        worker_id,
        control_open_latency_ms = duration_millis(control_open_started.elapsed()),
        "opened QUIC-native control stream"
    );
    peer_transport::client_handshake(
        &mut control,
        local_node_name(),
        peer_transport::PeerProtocol::QuicNative,
    )
    .await?;
    let session = client_negotiate(
        &mut control,
        RouteSessionSpec::new(
            route_id.to_string(),
            local_node_name(),
            peer_transport::default_features(),
            vec![peer_transport::PeerProtocol::QuicNative.to_string()],
        ),
    )
    .await?;
    let worker = Arc::new(ConnectionWorker::new(worker_id, connection));
    Ok(ConnectedWorker {
        worker,
        control,
        route_id: session.welcome.route_id,
        _session_node: session.hello.node,
    })
}
