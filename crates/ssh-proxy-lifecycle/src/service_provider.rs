pub mod contract;
pub mod kind;
pub mod remote;
pub mod selection;
pub mod status;

pub use contract::{PeerServiceProvider, ServiceProviderPlan};
pub use kind::ServiceProviderKind;
pub use remote::{
    ProviderActionPlan, RemotePeerServiceSpec, remote_auto_install_command,
    remote_launchd_install_command, remote_nohup_files, remote_nohup_start_command,
    remote_nohup_status_snippet, remote_nohup_stop_snippet, remote_schtasks_install_command,
    remote_service_action_plan, remote_systemd_install_command, remote_write_peer_artifact_command,
};
pub use selection::{
    provider_external_action_report, provider_for_platform, provider_for_remote_report,
};
pub use status::{ProviderStatus, ProviderStatusState};
