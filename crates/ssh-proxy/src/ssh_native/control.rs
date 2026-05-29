use std::{net::SocketAddr, sync::Arc};

use anyhow::{Context, Result};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};
use tracing::{info, warn};

use super::State;

pub(super) async fn run_control_server(addr: SocketAddr, state: Arc<State>) -> Result<()> {
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind ssh-native control listener {addr}"))?;
    info!(%addr, "ssh-native control listener ready");
    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, peer) = accept?;
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_control(stream, state).await {
                        warn!(%peer, error = %err, "ssh-native control request failed");
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
