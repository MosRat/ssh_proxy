use crate::peer_lifecycle;

use super::JobPhase;

pub(super) fn lifecycle_phase_from_job(
    phase: JobPhase,
) -> peer_lifecycle::workflow::PeerLifecyclePhase {
    match phase {
        JobPhase::InspectPeerDescriptor => {
            peer_lifecycle::workflow::PeerLifecyclePhase::InspectDescriptor
        }
        JobPhase::DependencyCheck => peer_lifecycle::workflow::PeerLifecyclePhase::DependencyCheck,
        JobPhase::StageRemotePeer => peer_lifecycle::workflow::PeerLifecyclePhase::StageBinary,
        JobPhase::WritePeerConfig => peer_lifecycle::workflow::PeerLifecyclePhase::WriteConfig,
        JobPhase::InstallPeerService => {
            peer_lifecycle::workflow::PeerLifecyclePhase::InstallService
        }
        JobPhase::StartPeerService => peer_lifecycle::workflow::PeerLifecyclePhase::StartService,
        JobPhase::PeerHealthProbe => peer_lifecycle::workflow::PeerLifecyclePhase::HealthProbe,
        JobPhase::RecordPeer => peer_lifecycle::workflow::PeerLifecyclePhase::Record,
        JobPhase::Failed => peer_lifecycle::workflow::PeerLifecyclePhase::Failed,
        _ => peer_lifecycle::workflow::PeerLifecyclePhase::Prepare,
    }
}

pub(super) fn job_phase_from_lifecycle(
    phase: peer_lifecycle::workflow::PeerLifecyclePhase,
) -> JobPhase {
    match phase {
        peer_lifecycle::workflow::PeerLifecyclePhase::InspectDescriptor => {
            JobPhase::InspectPeerDescriptor
        }
        peer_lifecycle::workflow::PeerLifecyclePhase::DependencyCheck => JobPhase::DependencyCheck,
        peer_lifecycle::workflow::PeerLifecyclePhase::StageBinary => JobPhase::StageRemotePeer,
        peer_lifecycle::workflow::PeerLifecyclePhase::WriteConfig => JobPhase::WritePeerConfig,
        peer_lifecycle::workflow::PeerLifecyclePhase::InstallService => {
            JobPhase::InstallPeerService
        }
        peer_lifecycle::workflow::PeerLifecyclePhase::StartService => JobPhase::StartPeerService,
        peer_lifecycle::workflow::PeerLifecyclePhase::HealthProbe => JobPhase::PeerHealthProbe,
        peer_lifecycle::workflow::PeerLifecyclePhase::Record => JobPhase::RecordPeer,
        peer_lifecycle::workflow::PeerLifecyclePhase::Failed => JobPhase::Failed,
        _ => JobPhase::Queued,
    }
}

pub(super) fn lifecycle_phase_from_install_state(
    install_state: &str,
    install_phase: &str,
) -> peer_lifecycle::workflow::PeerLifecyclePhase {
    if install_state == "healthy" {
        return peer_lifecycle::workflow::PeerLifecyclePhase::Healthy;
    }
    match install_phase {
        "inspect_descriptor" => peer_lifecycle::workflow::PeerLifecyclePhase::InspectDescriptor,
        "dependency_check" => peer_lifecycle::workflow::PeerLifecyclePhase::DependencyCheck,
        "stage_binary" => peer_lifecycle::workflow::PeerLifecyclePhase::StageBinary,
        "write_config" => peer_lifecycle::workflow::PeerLifecyclePhase::WriteConfig,
        "install_service" => peer_lifecycle::workflow::PeerLifecyclePhase::InstallService,
        "start_service" => peer_lifecycle::workflow::PeerLifecyclePhase::StartService,
        "health_probe" => peer_lifecycle::workflow::PeerLifecyclePhase::HealthProbe,
        "record_peer" | "record" => peer_lifecycle::workflow::PeerLifecyclePhase::Record,
        "failed" => peer_lifecycle::workflow::PeerLifecyclePhase::Failed,
        _ => peer_lifecycle::workflow::PeerLifecyclePhase::Prepare,
    }
}
