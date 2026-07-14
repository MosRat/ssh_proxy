use serde::{Deserialize, Serialize};

use super::{
    report::PeerLifecycleReport,
    service_provider::ServiceProviderKind,
    spec::{PeerLifecyclePlatform, PeerLifecycleRole, PeerLifecycleScope, PeerLifecycleSpec},
    workflow::{LifecycleOperation, PeerLifecyclePhase},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PeerLifecycleExecutorKind {
    Local,
    Ssh,
    Fake,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PeerLifecycleStoreKind {
    LocalDaemonState,
    RemotePeerState,
    Ephemeral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PeerLifecycleReportSinkKind {
    DaemonJobEvents,
    Vec,
    Stdout,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PeerLifecycleContext {
    pub(crate) target: String,
    pub(crate) role: PeerLifecycleRole,
    pub(crate) platform: PeerLifecyclePlatform,
    pub(crate) scope: PeerLifecycleScope,
    pub(crate) provider: ServiceProviderKind,
    pub(crate) service_name: String,
    pub(crate) state_dir: String,
    pub(crate) executor: PeerLifecycleExecutorKind,
    pub(crate) store: PeerLifecycleStoreKind,
    pub(crate) report_sink: PeerLifecycleReportSinkKind,
}

impl PeerLifecycleContext {
    pub(crate) fn from_spec(spec: &PeerLifecycleSpec) -> Self {
        Self {
            target: spec.target.clone(),
            role: spec.role,
            platform: spec.platform,
            scope: spec.scope,
            provider: spec.provider,
            service_name: spec.service_name.clone(),
            state_dir: spec.state_dir.clone(),
            executor: PeerLifecycleExecutorKind::Unknown,
            store: store_kind_for_role(spec.role),
            report_sink: PeerLifecycleReportSinkKind::Unknown,
        }
    }

    pub(crate) fn with_executor(mut self, executor: PeerLifecycleExecutorKind) -> Self {
        self.executor = executor;
        self
    }

    pub(crate) fn with_store(mut self, store: PeerLifecycleStoreKind) -> Self {
        self.store = store;
        self
    }

    pub(crate) fn with_report_sink(mut self, report_sink: PeerLifecycleReportSinkKind) -> Self {
        self.report_sink = report_sink;
        self
    }

    pub(crate) fn phase_report(
        &self,
        operation: LifecycleOperation,
        phase: PeerLifecyclePhase,
    ) -> PeerLifecycleReport {
        let spec = self.as_spec_view();
        let mut report = PeerLifecycleReport::new(self.target.clone(), phase);
        report.apply_spec(&spec, operation);
        report
    }

    fn as_spec_view(&self) -> PeerLifecycleSpec {
        PeerLifecycleSpec {
            role: self.role,
            target: self.target.clone(),
            platform: self.platform,
            scope: self.scope,
            provider: self.provider,
            service_name: self.service_name.clone(),
            binary_path: String::new(),
            transport: None,
            control_endpoint: None,
            token: None,
            state_dir: self.state_dir.clone(),
            rollback_policy: super::spec::RollbackPolicy::None,
        }
    }
}

fn store_kind_for_role(role: PeerLifecycleRole) -> PeerLifecycleStoreKind {
    match role {
        PeerLifecycleRole::LocalDaemon => PeerLifecycleStoreKind::LocalDaemonState,
        PeerLifecycleRole::RemotePeer => PeerLifecycleStoreKind::RemotePeerState,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_context_preserves_symmetric_spec_metadata() {
        let spec = PeerLifecycleSpec::local_daemon(
            "local",
            "ssh_proxy",
            ServiceProviderKind::SystemdUser,
            "ssh_proxy",
            Some("unix://socket".to_string()),
            None,
            None,
            "$HOME/.ssh_proxy",
        );

        let context = PeerLifecycleContext::from_spec(&spec)
            .with_executor(PeerLifecycleExecutorKind::Fake)
            .with_report_sink(PeerLifecycleReportSinkKind::Vec);
        let report =
            context.phase_report(LifecycleOperation::Ensure, PeerLifecyclePhase::HealthProbe);

        assert_eq!(context.role, PeerLifecycleRole::LocalDaemon);
        assert_eq!(context.store, PeerLifecycleStoreKind::LocalDaemonState);
        assert_eq!(context.executor, PeerLifecycleExecutorKind::Fake);
        assert_eq!(report.role.as_deref(), Some("local_daemon"));
        assert_eq!(report.provider.as_deref(), Some("systemd_user"));
        assert_eq!(report.service_name.as_deref(), Some("ssh_proxy"));
    }
}
