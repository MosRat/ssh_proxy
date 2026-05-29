use crate::{peer_lifecycle::report::PeerLifecycleReport, ssh_client::ExecOutput};

use super::{
    events::LifecycleEvent,
    model::{LifecycleOperation, PeerLifecyclePhase},
};

#[derive(Debug, Clone)]
pub(crate) enum LifecycleActionResult {
    Completed,
    CommandOutput(ExecOutput),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LifecycleFailure {
    pub(crate) phase: PeerLifecyclePhase,
    pub(crate) action: &'static str,
    pub(crate) blocker: String,
    pub(crate) message: String,
    pub(crate) exit_status: Option<u32>,
}

impl LifecycleFailure {
    pub(crate) fn from_command(
        phase: PeerLifecyclePhase,
        action: &'static str,
        output: &ExecOutput,
    ) -> Self {
        let message = if output.stderr.trim().is_empty() {
            output.stdout.trim().to_string()
        } else {
            output.stderr.trim().to_string()
        };
        Self {
            phase,
            action,
            blocker: format!("{}_failed", phase.as_str()),
            message,
            exit_status: Some(output.exit_status),
        }
    }

    pub(crate) fn from_error(
        phase: PeerLifecyclePhase,
        action: &'static str,
        error: impl Into<String>,
    ) -> Self {
        Self {
            phase,
            action,
            blocker: format!("{}_failed", phase.as_str()),
            message: error.into(),
            exit_status: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LifecycleStepStatus {
    Completed {
        phase: PeerLifecyclePhase,
        action: &'static str,
    },
    Failed(LifecycleFailure),
}

#[derive(Debug, Clone)]
pub(crate) struct PeerLifecycleWorkflowResult {
    pub(crate) operation: LifecycleOperation,
    pub(crate) report: PeerLifecycleReport,
    pub(crate) phase_reports: Vec<PeerLifecycleReport>,
    pub(crate) events: Vec<LifecycleEvent>,
    pub(crate) step_statuses: Vec<LifecycleStepStatus>,
    pub(crate) redacted_report: serde_json::Value,
}
