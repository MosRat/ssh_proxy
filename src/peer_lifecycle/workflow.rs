use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PeerLifecyclePhase {
    Prepare,
    InspectDescriptor,
    DependencyCheck,
    StageBinary,
    WriteConfig,
    InstallService,
    StartService,
    HealthProbe,
    Record,
    Healthy,
    Repairing,
    Rollback,
    Failed,
}

impl PeerLifecyclePhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::InspectDescriptor => "inspect_descriptor",
            Self::DependencyCheck => "dependency_check",
            Self::StageBinary => "stage_binary",
            Self::WriteConfig => "write_config",
            Self::InstallService => "install_service",
            Self::StartService => "start_service",
            Self::HealthProbe => "health_probe",
            Self::Record => "record",
            Self::Healthy => "healthy",
            Self::Repairing => "repairing",
            Self::Rollback => "rollback",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn progress(self) -> u8 {
        match self {
            Self::Prepare => 5,
            Self::InspectDescriptor => 15,
            Self::DependencyCheck => 25,
            Self::StageBinary => 35,
            Self::WriteConfig => 45,
            Self::InstallService => 60,
            Self::StartService => 72,
            Self::HealthProbe => 85,
            Self::Record => 95,
            Self::Healthy => 100,
            Self::Repairing => 50,
            Self::Rollback => 90,
            Self::Failed => 100,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_phase_names_match_public_json_contract() {
        assert_eq!(PeerLifecyclePhase::Prepare.as_str(), "prepare");
        assert_eq!(
            PeerLifecyclePhase::InspectDescriptor.as_str(),
            "inspect_descriptor"
        );
        assert_eq!(PeerLifecyclePhase::HealthProbe.progress(), 85);
        assert_eq!(PeerLifecyclePhase::Failed.progress(), 100);
    }
}
