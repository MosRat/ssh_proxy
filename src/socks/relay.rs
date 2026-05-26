use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use anyhow::{Context, Result};
use bytes::{Bytes, BytesMut};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional_with_sizes},
    net::TcpStream,
    sync::watch,
    time,
};
use tracing::debug;

use crate::{controller, data_plane, protocol::TCP_DATA_CHUNK, quic_native, ssh_native};

enum QuicNativeRelayOutcome {
    Completed((u64, u64)),
    FirstByteTimeout,
    CopyFailed(anyhow::Error),
}

pub(super) async fn relay_spx_tcp(
    stream: TcpStream,
    initial_remote: Vec<u8>,
    remote_flow: data_plane::SpxTcpFlow,
    state: Arc<controller::SharedState>,
    worker_slot: usize,
) -> Result<()> {
    let (remote_tx, mut remote_rx, remote_close) = remote_flow.split();
    let (mut client_reader, mut client_writer) = stream.into_split();
    let relay_started = Instant::now();
    let relay_client_to_remote = Arc::new(AtomicU64::new(0));
    let relay_remote_to_client = Arc::new(AtomicU64::new(0));
    let state_to_remote = state.clone();
    let client_to_remote_bytes = relay_client_to_remote.clone();
    let mut client_to_remote = tokio::spawn(async move {
        if !initial_remote.is_empty() {
            state_to_remote.record_client_to_remote_bytes(initial_remote.len());
            state_to_remote.record_worker_client_to_remote_bytes(worker_slot, initial_remote.len());
            client_to_remote_bytes.fetch_add(initial_remote.len() as u64, Ordering::Relaxed);
            remote_tx.send(Bytes::from(initial_remote)).await?;
        }
        let mut buf = BytesMut::with_capacity(TCP_DATA_CHUNK);
        loop {
            if buf.capacity() < TCP_DATA_CHUNK {
                buf.reserve(TCP_DATA_CHUNK - buf.capacity());
            }
            let n = client_reader.read_buf(&mut buf).await?;
            if n == 0 {
                break;
            }
            state_to_remote.record_client_to_remote_bytes(n);
            state_to_remote.record_worker_client_to_remote_bytes(worker_slot, n);
            client_to_remote_bytes.fetch_add(n as u64, Ordering::Relaxed);
            remote_tx.send(buf.split().freeze()).await?;
        }
        Ok::<_, anyhow::Error>(())
    });

    let state_to_client = state.clone();
    let remote_to_client_bytes = relay_remote_to_client.clone();
    let mut remote_to_client = tokio::spawn(async move {
        while let Some(data) = remote_rx.recv().await {
            state_to_client.record_remote_to_client_bytes(data.len());
            state_to_client.record_worker_remote_to_client_bytes(worker_slot, data.len());
            remote_to_client_bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
            client_writer.write_all(&data).await?;
        }
        client_writer.shutdown().await?;
        Ok::<_, anyhow::Error>(())
    });

    let close_reason = tokio::select! {
        result = &mut client_to_remote => {
            let reason = match flatten_join(result) {
                Ok(()) => "client-to-remote completed".to_string(),
                Err(err) => {
                    debug!(error = %err, "client-to-remote TCP task ended");
                    format!("client-to-remote error: {err}")
                }
            };
            remote_to_client.abort();
            reason
        }
        result = &mut remote_to_client => {
            let reason = match flatten_join(result) {
                Ok(()) => "remote-to-client completed".to_string(),
                Err(err) => {
                    debug!(error = %err, "remote-to-client TCP task ended");
                    format!("remote-to-client error: {err}")
                }
            };
            client_to_remote.abort();
            reason
        }
    };
    state
        .record_spx_tcp_relay(
            relay_started.elapsed(),
            relay_client_to_remote.load(Ordering::Relaxed) as usize,
            relay_remote_to_client.load(Ordering::Relaxed) as usize,
            close_reason,
        )
        .await;
    remote_close.close("tcp connection closed").await;
    Ok(())
}

pub(super) async fn relay_ssh_native_tcp(
    mut stream: TcpStream,
    mut remote: ssh_native::Stream,
    initial_remote: Vec<u8>,
    _state: Arc<ssh_native::State>,
) -> Result<()> {
    if !initial_remote.is_empty() {
        if let Err(err) = remote.write_all(&initial_remote).await {
            remote.record_error_close(format!("initial client bytes write failed: {err}"));
            return Err(err.into());
        }
        remote.record_client_to_remote_bytes(initial_remote.len());
    }

    let result =
        copy_bidirectional_with_sizes(&mut stream, &mut remote, TCP_DATA_CHUNK, TCP_DATA_CHUNK)
            .await;
    let (client_to_remote, remote_to_client) = match result {
        Ok(counts) => counts,
        Err(err) => {
            remote.record_error_close(format!("tcp relay copy failed: {err}"));
            return Err(err.into());
        }
    };
    remote.record_client_to_remote_bytes(client_to_remote as usize);
    remote.record_remote_to_client_bytes(remote_to_client as usize);
    remote.record_graceful_close("tcp relay completed");
    if let Err(err) = remote.shutdown().await {
        debug!(error = %err, "ssh-native remote shutdown failed after relay close");
    }
    Ok(())
}

pub(super) async fn relay_quic_native_tcp(
    stream: TcpStream,
    mut remote: quic_native::Stream,
    initial_remote: Vec<u8>,
    state: Arc<quic_native::State>,
) -> Result<()> {
    let relay_started = Instant::now();
    let initial_remote_len = initial_remote.len() as u64;
    if !initial_remote.is_empty() {
        match time::timeout(
            quic_native::QUIC_NATIVE_BACKPRESSURE_TIMEOUT,
            remote.write_all(&initial_remote),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                state.record_quic_copy_failure();
                remote
                    .reset(format!("initial client bytes write failed: {err}"))
                    .await;
                return Err(err.into());
            }
            Err(_) => {
                state.record_quic_copy_failure();
                state.record_quic_backpressure_timeout();
                remote.record_backpressure_timeout();
                remote
                    .reset(format!(
                        "initial client bytes write timed out after {}s",
                        quic_native::QUIC_NATIVE_BACKPRESSURE_TIMEOUT.as_secs()
                    ))
                    .await;
                anyhow::bail!(
                    "initial client bytes write timed out after {}s",
                    quic_native::QUIC_NATIVE_BACKPRESSURE_TIMEOUT.as_secs()
                );
            }
        }
        state.record_client_to_remote_bytes(initial_remote.len());
    }

    let first_byte_recorded = remote.first_byte_recorded();
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let timeout_state = state.clone();
    let watchdog = tokio::spawn({
        let first_byte_recorded = first_byte_recorded.clone();
        async move {
            time::sleep(quic_native::QUIC_NATIVE_FIRST_BYTE_TIMEOUT).await;
            if !first_byte_recorded.load(Ordering::Relaxed) {
                let _ = cancel_tx.send(true);
                debug!(
                    timeout_secs = quic_native::QUIC_NATIVE_FIRST_BYTE_TIMEOUT.as_secs(),
                    "QUIC-native relay first byte timeout fired"
                );
                timeout_state.record_quic_copy_failure();
            }
        }
    });

    let mut stream = stream;
    let mut remote = remote;
    let mut cancel_rx = cancel_rx;
    let outcome = tokio::select! {
        result = copy_bidirectional_with_sizes(
            &mut stream,
            &mut remote,
            quic_native::QUIC_NATIVE_COPY_BUFFER_SIZE,
            quic_native::QUIC_NATIVE_COPY_BUFFER_SIZE,
        ) => match result {
            Ok(counts) => QuicNativeRelayOutcome::Completed(counts),
            Err(err) => QuicNativeRelayOutcome::CopyFailed(err.into()),
        },
        _ = cancel_rx.changed() => {
            remote.reset(format!(
                "QUIC-native first byte timed out after {}s",
                quic_native::QUIC_NATIVE_FIRST_BYTE_TIMEOUT.as_secs()
            )).await;
            QuicNativeRelayOutcome::FirstByteTimeout
        }
    };
    watchdog.abort();
    match outcome {
        QuicNativeRelayOutcome::Completed((client_to_remote, remote_to_client)) => {
            state.record_client_to_remote_bytes(client_to_remote as usize);
            state.record_remote_to_client_bytes(remote_to_client as usize);
            state.record_quic_copy(
                relay_started.elapsed(),
                initial_remote_len.saturating_add(client_to_remote),
                remote_to_client,
            );
            remote.finish("tcp relay completed").await;
            Ok(())
        }
        QuicNativeRelayOutcome::FirstByteTimeout => Err(anyhow::anyhow!(
            "QUIC-native first byte timed out after {}s",
            quic_native::QUIC_NATIVE_FIRST_BYTE_TIMEOUT.as_secs()
        )),
        QuicNativeRelayOutcome::CopyFailed(err) => {
            state.record_quic_copy_failure();
            remote.reset(format!("copy failed: {err}")).await;
            Err(err)
        }
    }
}

fn flatten_join(result: std::result::Result<Result<()>, tokio::task::JoinError>) -> Result<()> {
    result.context("task panicked")?
}
