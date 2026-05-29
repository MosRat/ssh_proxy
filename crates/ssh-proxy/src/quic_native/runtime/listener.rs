use std::{net::SocketAddr, sync::Arc};

use anyhow::{Context, Result, anyhow};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};
use tracing::{debug, info, warn};

use crate::{cli, socks};

use super::{State, StateSlot};

pub async fn run_with_state(args: cli::ProxyArgs, state: Arc<State>) -> Result<()> {
    if let Some(addr) = args.control_listen {
        let control_state = state.clone();
        tokio::spawn(async move {
            if let Err(err) = run_control_server(addr, control_state).await {
                warn!(%addr, error = %err, "quic-native control server stopped");
            }
        });
    }

    let listener = TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("failed to bind quic-native proxy listener {}", args.listen))?;
    info!(listen = %args.listen, "quic-native proxy listening");

    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, peer) = accept?;
                let _ = stream.set_nodelay(true);
                let state = state.clone();
                let tcp_target = args.tcp_target.clone();
                tokio::spawn(async move {
                    let result = if let Some(target) = tcp_target {
                        socks::handle_fixed_target_quic_native(stream, peer, target, state).await
                    } else {
                        socks::handle_client_quic_native(stream, peer, state).await
                    };
                    if let Err(err) = result {
                        debug!(%peer, error = %err, "quic-native proxy client failed");
                    }
                });
            }
            _ = state.shutdown_notified() => {
                info!("shutdown requested; stopping quic-native proxy listener");
                break;
            }
        }
    }
    if let Some(err) = state.shutdown_error().await {
        return Err(anyhow!("quic-native control stream degraded: {err}"));
    }
    Ok(())
}

pub async fn run_with_slot(args: cli::ProxyArgs, slot: Arc<StateSlot>) -> Result<()> {
    let state = State::connect(args.clone()).await?;
    slot.set_current(state.clone()).await;
    let result = run_with_state(args, state.clone()).await;
    let err = result.as_ref().err().map(|err| err.to_string());
    slot.clear_current(&state, err).await;
    result
}

async fn run_control_server(addr: SocketAddr, state: Arc<State>) -> Result<()> {
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind quic-native control listener {addr}"))?;
    info!(%addr, "quic-native control listener ready");
    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, peer) = accept?;
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_control(stream, state).await {
                        warn!(%peer, error = %err, "quic-native control request failed");
                    }
                });
            }
            _ = state.shutdown_notified() => break,
        }
    }
    Ok(())
}

async fn handle_control(stream: TcpStream, state: Arc<State>) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut command = String::new();
    reader.read_line(&mut command).await?;
    match command.trim().to_ascii_lowercase().as_str() {
        "status" | "" => {
            writer
                .write_all(state.status_json().await?.as_bytes())
                .await?
        }
        "shutdown" => {
            state.request_shutdown();
            writer
                .write_all(b"{\"ok\":true,\"message\":\"shutdown requested\"}\n")
                .await?;
        }
        other => {
            let response = serde_json::json!({
                "ok": false,
                "error": format!("unknown command {other:?}; expected status or shutdown")
            });
            writer
                .write_all(format!("{}\n", serde_json::to_string_pretty(&response)?).as_bytes())
                .await?;
        }
    }
    writer.shutdown().await.ok();
    Ok(())
}
