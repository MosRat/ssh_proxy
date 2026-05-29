use std::{
    sync::{Arc, atomic::Ordering},
    time::Instant,
};

use anyhow::{Result, anyhow};
use tokio::time;
use tracing::debug;

use crate::{quic_native, quic_stream};

use super::{
    CONTROL_KEEPALIVE_INTERVAL, CONTROL_KEEPALIVE_TIMEOUT, ConnectionWorker, State, duration_millis,
};

pub(super) async fn run_control_loop(
    mut stream: quic_stream::QuicBiStream,
    state: Arc<State>,
    worker: Arc<ConnectionWorker>,
) -> Result<()> {
    let mut keepalive = time::interval(CONTROL_KEEPALIVE_INTERVAL);
    keepalive.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    let mut next_ping = 1_u64;
    let mut pending_ping: Option<(u64, Instant)> = None;
    loop {
        tokio::select! {
            frame = quic_native::control::RouteControlFrame::read_from(&mut stream) => {
                let frame = match frame {
                    Ok(frame) => frame,
                    Err(err) => {
                        worker.mark_control_degraded(err.to_string()).await;
                        *state.last_control_error.lock().await = Some(err.to_string());
                        state.control_degraded.store(true, Ordering::Relaxed);
                        return Err(err);
                    }
                };
                match frame {
                    quic_native::control::RouteControlFrame::Ping { seq } => {
                        if let Err(err) = (quic_native::control::RouteControlFrame::Pong { seq })
                            .write_to(&mut stream)
                            .await
                        {
                            worker.mark_control_degraded(err.to_string()).await;
                            *state.last_control_error.lock().await = Some(err.to_string());
                            state.control_degraded.store(true, Ordering::Relaxed);
                            return Err(err);
                        }
                    }
                    quic_native::control::RouteControlFrame::Pong { seq } => {
                        if let Some((pending_seq, sent_at)) = pending_ping {
                            if pending_seq == seq {
                                debug!(
                                    seq,
                                    pong_latency_ms = duration_millis(sent_at.elapsed()),
                                    "received QUIC-native control pong"
                                );
                                worker
                                    .control_pongs_received
                                    .fetch_add(1, Ordering::Relaxed);
                                worker
                                    .last_control_pong_latency_ms
                                    .store(duration_millis(sent_at.elapsed()), Ordering::Relaxed);
                                state.control_pongs_received.fetch_add(1, Ordering::Relaxed);
                                state
                                    .last_control_pong_latency_ms
                                    .store(duration_millis(sent_at.elapsed()), Ordering::Relaxed);
                                pending_ping = None;
                            }
                        }
                    }
                    quic_native::control::RouteControlFrame::Hello(_)
                    | quic_native::control::RouteControlFrame::Welcome(_) => {}
                }
            }
            _ = keepalive.tick() => {
                if let Some((seq, sent_at)) = pending_ping {
                    if sent_at.elapsed() >= CONTROL_KEEPALIVE_TIMEOUT {
                        let err = format!(
                            "QUIC-native control pong {seq} timed out after {}s",
                            CONTROL_KEEPALIVE_TIMEOUT.as_secs()
                        );
                        worker.mark_control_degraded(err.clone()).await;
                        *state.last_control_error.lock().await = Some(err.clone());
                        state.control_degraded.store(true, Ordering::Relaxed);
                        return Err(anyhow!(err));
                    }
                }
                if pending_ping.is_none() {
                    let seq = next_ping;
                    next_ping = next_ping.saturating_add(1);
                    if let Err(err) = (quic_native::control::RouteControlFrame::Ping { seq })
                        .write_to(&mut stream)
                        .await
                    {
                        worker.mark_control_degraded(err.to_string()).await;
                        *state.last_control_error.lock().await = Some(err.to_string());
                        state.control_degraded.store(true, Ordering::Relaxed);
                        return Err(err);
                    }
                    worker.control_pings_sent.fetch_add(1, Ordering::Relaxed);
                    state.control_pings_sent.fetch_add(1, Ordering::Relaxed);
                    pending_ping = Some((seq, Instant::now()));
                    debug!(seq, "sent QUIC-native control ping");
                }
            }
        }
    }
}
