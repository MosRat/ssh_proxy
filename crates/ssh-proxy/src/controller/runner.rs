use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use tokio::net::TcpListener;
use tracing::{debug, error, info, warn};

use crate::{cli, peer_transport, quic_native, socks, ssh_native};

use super::{SharedState, bridge_manager, listener};

pub async fn run(args: cli::ProxyArgs) -> Result<()> {
    match args.remote_transport {
        cli::RemoteTransport::SshNative => {
            let state = ssh_native::State::new(args.clone());
            run_ssh_native_with_state(args, state).await
        }
        cli::RemoteTransport::QuicNative => {
            let state = quic_native::State::connect(args.clone()).await?;
            run_quic_native_with_state(args, state).await
        }
        _ => {
            let state = shared_state(&args);
            run_with_state(args, state).await
        }
    }
}

pub async fn run_ssh_native_with_state(
    args: cli::ProxyArgs,
    state: Arc<ssh_native::State>,
) -> Result<()> {
    ssh_native::run_with_state(args, state).await
}

pub async fn run_quic_native_with_state(
    args: cli::ProxyArgs,
    state: Arc<quic_native::State>,
) -> Result<()> {
    quic_native::run_with_state(args, state).await
}

pub async fn run_quic_native_with_slot(
    args: cli::ProxyArgs,
    slot: Arc<quic_native::StateSlot>,
) -> Result<()> {
    quic_native::run_with_slot(args, slot).await
}

pub fn shared_state(args: &cli::ProxyArgs) -> Arc<SharedState> {
    let quic_options = peer_transport::QuicTransportOptions::new(
        args.quic_max_bidi_streams,
        args.quic_stream_receive_window,
        args.quic_receive_window,
        args.quic_keep_alive_interval_secs,
        args.quic_idle_timeout_secs,
    )
    .unwrap_or_default();
    Arc::new(SharedState::new(
        !args.no_reconnect,
        args.egress_proxy.clone(),
        args.transport_pool_size,
        quic_options,
        tls_peer_auth_mode(args),
        args.pool_policy.clone(),
        args.workload_hint
            .map(workload_hint_name)
            .map(str::to_string),
    ))
}

pub async fn run_with_state(args: cli::ProxyArgs, state: Arc<SharedState>) -> Result<()> {
    if matches!(
        args.remote_transport,
        cli::RemoteTransport::Auto | cli::RemoteTransport::Exec
    ) {
        warn!(
            "proxy may use a temporary SSH exec helper; for production prefer `ssh_proxy daemon install --scope system --elevate` locally and `ssh_proxy up ...` for daemon-owned sessions"
        );
    }
    let tcp_target = args.tcp_target.clone();
    let manager_state = state.clone();
    let manager_args = args.clone();
    tokio::spawn(async move {
        bridge_manager::run(manager_args, manager_state).await;
    });
    if args.no_reconnect {
        state
            .wait_for_initial_bridge(Duration::from_secs(args.connect_timeout_secs.max(1)))
            .await?;
    }

    if let Some(addr) = args.control_listen {
        let control_state = state.clone();
        tokio::spawn(async move {
            if let Err(err) = listener::run_control_server(addr, control_state).await {
                error!(%addr, error = %err, "control server stopped");
            }
        });
    }

    let listener = TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("failed to bind SOCKS listener {}", args.listen))?;
    info!(listen = %args.listen, "SOCKS5H proxy listening");

    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, peer) = accept?;
                let state = state.clone();
                let tcp_target = tcp_target.clone();
                tokio::spawn(async move {
                    let result = if let Some(target) = tcp_target {
                        socks::handle_fixed_target(stream, peer, target, state).await
                    } else {
                        socks::handle_client(stream, peer, state).await
                    };
                    if let Err(err) = result {
                        debug!(%peer, error = %err, "SOCKS client failed");
                    }
                });
            }
            _ = state.shutdown_notified() => {
                info!("shutdown requested; stopping SOCKS listener");
                break;
            }
        }
    }
    Ok(())
}

fn tls_peer_auth_mode(args: &cli::ProxyArgs) -> Option<String> {
    if !matches!(args.remote_transport, cli::RemoteTransport::TlsTcp) {
        return None;
    }
    Some(
        match (
            args.remote_client_cert.is_some(),
            args.remote_client_key.is_some(),
        ) {
            (true, true) => "mutual_tls",
            (false, false) => "server_auth",
            _ => "invalid_client_auth_config",
        }
        .to_string(),
    )
}

fn workload_hint_name(hint: cli::RouteWorkloadHint) -> &'static str {
    match hint {
        cli::RouteWorkloadHint::Large => "large",
        cli::RouteWorkloadHint::Concurrent => "concurrent",
        cli::RouteWorkloadHint::Mixed => "mixed",
    }
}
