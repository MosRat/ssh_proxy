use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use anyhow::{Result, anyhow};
use tokio::time;
use tracing::{error, info, warn};

use crate::{bridge, cli, deploy};

use super::{SharedState, duration_millis};

pub(super) async fn run(args: cli::ProxyArgs, state: Arc<SharedState>) {
    let pool_size = args.transport_pool_size.max(1);
    info!(pool_size, "starting route peer transport pool");
    let mut workers = Vec::with_capacity(pool_size);
    for slot in 0..pool_size {
        let args = args.clone();
        let state = state.clone();
        workers.push(tokio::spawn(async move {
            run_worker(slot, args, state).await;
        }));
    }
    for worker in workers {
        worker.await.ok();
    }
}

async fn run_worker(slot: usize, args: cli::ProxyArgs, state: Arc<SharedState>) {
    let mut delay = Duration::from_secs(args.reconnect_delay_secs);
    let max_delay =
        Duration::from_secs(args.reconnect_max_delay_secs.max(args.reconnect_delay_secs));
    loop {
        if state.shutdown.load(Ordering::Relaxed) {
            break;
        }
        let attempt = state.record_bridge_attempt(slot).await;
        info!(slot, attempt, "connecting remote bridge");
        let connect = time::timeout(
            Duration::from_secs(args.connect_timeout_secs.max(1)),
            bridge::Bridge::connect_via_ssh(&args),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "remote bridge connection timed out after {}s",
                args.connect_timeout_secs
            )
        })
        .and_then(|result| result);
        match connect {
            Ok(bridge) => {
                let generation = state.generation.fetch_add(1, Ordering::Relaxed) + 1;
                let selected_protocol = bridge.selected_protocol;
                let transport_timings = bridge.transport_timings;
                state.set_candidate_failures(Vec::new()).await;
                state
                    .record_bridge_connected(slot, generation, selected_protocol, transport_timings)
                    .await;
                delay = Duration::from_secs(args.reconnect_delay_secs);
                info!(slot, generation, "remote bridge connected");
                state.set_bridge(slot, Some(bridge.handle.clone())).await;
                tokio::select! {
                    _ = bridge.lifecycle => {}
                    _ = state.shutdown_notified() => {}
                }
                warn!(slot, generation, "remote bridge disconnected");
                state.record_bridge_disconnected(slot).await;
                state.set_bridge(slot, None).await;
            }
            Err(err) => {
                let candidate_failures = err
                    .downcast_ref::<deploy::AutoTransportError>()
                    .map(|err| err.failures.clone());
                let detail = format!("{err:#}");
                state.record_bridge_failed(slot, detail.clone()).await;
                if let Some(failures) = candidate_failures {
                    state.set_candidate_failures(failures).await;
                }
                error!(slot, attempt, error = %detail, "failed to connect remote bridge");
                state.set_bridge(slot, None).await;
            }
        }

        if !state.reconnect {
            info!("reconnect disabled; bridge manager exiting");
            break;
        }
        let sleep_delay = jittered_backoff(delay, max_delay);
        warn!(
            slot,
            base_retry_secs = delay.as_secs(),
            next_retry_secs = sleep_delay.as_secs_f64(),
            "retrying remote bridge connection after jittered backoff"
        );
        tokio::select! {
            _ = time::sleep(sleep_delay) => {}
            _ = state.shutdown_notified() => break,
        }
        delay = (delay * 2).min(max_delay);
    }
}

fn jittered_backoff(base: Duration, max_delay: Duration) -> Duration {
    let base_ms = duration_millis(base).max(1);
    let max_ms = duration_millis(max_delay).max(base_ms);
    let jitter_range_ms = (base_ms / 4).max(1);
    let seed = random_u64().unwrap_or_else(|| base_ms.rotate_left(13) ^ max_ms);
    let offset_ms = seed % (jitter_range_ms + 1);
    let jittered_ms = if seed & 1 == 0 {
        base_ms.saturating_add(offset_ms).min(max_ms)
    } else {
        base_ms.saturating_sub(offset_ms).max(1)
    };
    Duration::from_millis(jittered_ms)
}

fn random_u64() -> Option<u64> {
    let mut bytes = [0_u8; 8];
    getrandom::fill(&mut bytes).ok()?;
    Some(u64::from_le_bytes(bytes))
}
