use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::net::TcpListener;
use tracing::{debug, info, warn};

use crate::{cli, socks};

use super::{State, control};

pub async fn run_with_state(args: cli::ProxyArgs, state: Arc<State>) -> Result<()> {
    if let Some(addr) = args.control_listen {
        let control_state = state.clone();
        tokio::spawn(async move {
            if let Err(err) = control::run_control_server(addr, control_state).await {
                warn!(%addr, error = %err, "ssh-native control server stopped");
            }
        });
    }

    let listener = TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("failed to bind ssh-native proxy listener {}", args.listen))?;
    info!(listen = %args.listen, "ssh-native proxy listening");

    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, peer) = accept?;
                let state = state.clone();
                let tcp_target = args.tcp_target.clone();
                tokio::spawn(async move {
                    let result = if let Some(target) = tcp_target {
                        socks::handle_fixed_target_ssh_native(stream, peer, target, state).await
                    } else {
                        socks::handle_client_ssh_native(stream, peer, state).await
                    };
                    if let Err(err) = result {
                        debug!(%peer, error = %err, "ssh-native proxy client failed");
                    }
                });
            }
            _ = state.shutdown_notified() => {
                info!("shutdown requested; stopping ssh-native proxy listener");
                break;
            }
        }
    }
    Ok(())
}
