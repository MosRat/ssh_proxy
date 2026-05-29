mod events;
mod model;
mod outcome;
mod runner;

pub(crate) use events::{
    BoxEventFuture, LifecycleEvent, LifecycleEventSink, VecLifecycleEventSink,
};
#[allow(unused_imports)]
pub(crate) use model::{
    LifecycleAction, LifecycleCommand, LifecycleCommandPlan, LifecycleOperation, LifecyclePlan,
    LifecycleStep, PeerLifecyclePhase,
};
#[allow(unused_imports)]
pub(crate) use outcome::{
    LifecycleActionResult, LifecycleFailure, LifecycleStepStatus, PeerLifecycleWorkflowResult,
};
#[allow(unused_imports)]
pub(crate) use runner::{
    phase_report, phase_report_for_operation, run_lifecycle_commands, run_lifecycle_plan,
};

#[cfg(test)]
mod tests;
