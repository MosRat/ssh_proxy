use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

pub(super) fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

pub(super) fn last_sampled_u64(sample_count: u64, value: u64) -> Option<u64> {
    (sample_count > 0).then_some(value)
}

pub(super) fn record_latency_sample(
    samples: &AtomicU64,
    last: &AtomicU64,
    max: &AtomicU64,
    duration: Duration,
) {
    let millis = duration_millis(duration);
    samples.fetch_add(1, Ordering::Relaxed);
    last.store(millis, Ordering::Relaxed);
    update_max(max, millis);
}

pub(super) fn update_max(max: &AtomicU64, value: u64) {
    let mut current = max.load(Ordering::Relaxed);
    while value > current {
        match max.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(existing) => current = existing,
        }
    }
}
