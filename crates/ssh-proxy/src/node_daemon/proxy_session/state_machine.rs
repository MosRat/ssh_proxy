use super::super::jobs::{JobPhase, JobRecord, JobState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProxySessionStep {
    pub(super) phase: JobPhase,
    pub(super) progress: u8,
    pub(super) message: &'static str,
}

impl ProxySessionStep {
    const fn new(phase: JobPhase, progress: u8, message: &'static str) -> Self {
        Self {
            phase,
            progress,
            message,
        }
    }
}

pub(super) fn resolve_target_step() -> ProxySessionStep {
    ProxySessionStep::new(JobPhase::ResolveTarget, 10, "resolved proxy session target")
}

pub(super) fn validate_local_proxy_step() -> ProxySessionStep {
    ProxySessionStep::new(
        JobPhase::ValidateLocalProxy,
        18,
        "validated local proxy URL",
    )
}

pub(super) fn select_remote_port_step() -> ProxySessionStep {
    ProxySessionStep::new(
        JobPhase::SelectRemotePort,
        24,
        "selected preferred remote port",
    )
}

pub(super) fn ensure_peer_step() -> ProxySessionStep {
    ProxySessionStep::new(JobPhase::EnsurePeer, 35, "ensuring persistent remote peer")
}

pub(super) fn ensure_transport_step() -> ProxySessionStep {
    ProxySessionStep::new(
        JobPhase::EnsureTransport,
        45,
        "selected Rust transport strategy",
    )
}

pub(super) fn plan_route_step() -> ProxySessionStep {
    ProxySessionStep::new(JobPhase::PlanRoute, 50, "planned daemon-owned route")
}

pub(super) fn route_start_conflict_is_repairable(error: &str) -> bool {
    error.contains("is already running with a different spec")
}

pub(super) fn route_start_blocker(error: &str) -> String {
    if route_start_conflict_is_repairable(error) {
        "route_already_running_different_spec".to_string()
    } else if error.contains("SSH authentication failed") {
        "ssh_auth_failed".to_string()
    } else if error.contains("ProxyCommand")
        || error.contains("unsupported --ssh-arg")
        || error.contains("unsupported -o")
    {
        "ssh_config_unsupported".to_string()
    } else {
        "route_start_failed".to_string()
    }
}

pub(super) fn reusable_proxy_session_job(job: &JobRecord, live_route: bool) -> bool {
    match job.state {
        JobState::Queued | JobState::Running | JobState::WaitingRetry => true,
        JobState::Healthy => live_route,
        JobState::Failed | JobState::Cancelled => false,
    }
}

pub(super) fn job_health(job: &JobRecord) -> &'static str {
    match job.state {
        JobState::Healthy => "healthy",
        JobState::Failed => "failed",
        JobState::Cancelled => "cancelled",
        JobState::Queued | JobState::Running | JobState::WaitingRetry => "starting",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_start_blockers_are_stable() {
        assert_eq!(
            route_start_blocker("route is already running with a different spec"),
            "route_already_running_different_spec"
        );
        assert_eq!(
            route_start_blocker("SSH authentication failed"),
            "ssh_auth_failed"
        );
        assert_eq!(
            route_start_blocker("unsupported -o ProxyCommand"),
            "ssh_config_unsupported"
        );
    }

    #[test]
    fn proxy_session_steps_keep_public_progress_contract() {
        assert_eq!(resolve_target_step().phase, JobPhase::ResolveTarget);
        assert_eq!(validate_local_proxy_step().progress, 18);
        assert_eq!(
            ensure_peer_step().message,
            "ensuring persistent remote peer"
        );
    }
}
