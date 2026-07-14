use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use super::{Session, State};

pub(super) struct RuntimeCounterGuard {
    state: Arc<State>,
    session: Arc<Session>,
    active: bool,
}

impl RuntimeCounterGuard {
    pub(super) fn ssh_channel(state: Arc<State>, session: Arc<Session>) -> Self {
        Self {
            state,
            session,
            active: true,
        }
    }

    pub(super) fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for RuntimeCounterGuard {
    fn drop(&mut self) {
        if self.active {
            self.session.active_channels.fetch_sub(1, Ordering::Relaxed);
            self.state
                .active_ssh_channels
                .fetch_sub(1, Ordering::Relaxed);
        }
    }
}

pub(super) fn update_atomic_max(value: &AtomicU64, candidate: u64) {
    let mut current = value.load(Ordering::Relaxed);
    while candidate > current {
        match value.compare_exchange_weak(current, candidate, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}
