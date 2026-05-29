use std::{future::Future, pin::Pin};

use crate::peer_lifecycle::report::PeerLifecycleReport;

use super::model::LifecycleOperation;

#[derive(Debug, Clone)]
pub(crate) struct LifecycleEvent {
    pub(crate) operation: LifecycleOperation,
    pub(crate) report: PeerLifecycleReport,
    pub(crate) message: String,
}

pub(crate) type BoxEventFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

pub(crate) trait LifecycleEventSink {
    fn emit<'a>(&'a mut self, event: LifecycleEvent) -> BoxEventFuture<'a>;
}

#[derive(Debug, Default)]
pub(crate) struct VecLifecycleEventSink {
    pub(crate) events: Vec<LifecycleEvent>,
}

impl LifecycleEventSink for VecLifecycleEventSink {
    fn emit<'a>(&'a mut self, event: LifecycleEvent) -> BoxEventFuture<'a> {
        Box::pin(async move {
            self.events.push(event);
        })
    }
}
