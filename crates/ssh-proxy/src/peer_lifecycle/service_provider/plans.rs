use ssh_proxy_core::{
    intent::RemoteInstallIntent,
    model::{PersistenceMode, RemotePlatform},
};
use ssh_proxy_lifecycle::service_provider::{RemotePeerServiceSpec, remote_service_action_plan};

use super::{
    contract::ServiceProviderPlan, kind::ServiceProviderKind, selection::provider_for_platform,
};
use crate::peer_lifecycle::workflow::{LifecycleOperation, LifecyclePlan};

#[derive(Debug, Clone)]
pub(crate) struct RemoteServiceInstallPlan {
    pub(crate) provider: ServiceProviderPlan,
    pub(crate) command: String,
    pub(crate) action_plan: LifecyclePlan,
    pub(crate) reported_service_manager: String,
}

pub(crate) fn remote_service_install_plan(
    remote_path: &str,
    intent: &RemoteInstallIntent,
) -> RemoteServiceInstallPlan {
    let kind = provider_for_platform(intent.remote_platform, intent.persistence);
    let service_spec = RemotePeerServiceSpec::from_intent(remote_path, intent);
    let action_plan = remote_service_action_plan(&service_spec, LifecycleOperation::Install);
    let command = action_plan.command.clone();
    let reported_service_manager = match intent.persistence {
        PersistenceMode::None => "none",
        PersistenceMode::Auto if intent.remote_platform == RemotePlatform::Windows => {
            ServiceProviderKind::WindowsScheduledTaskUser.manager_name()
        }
        PersistenceMode::Auto => "auto",
        _ => kind.manager_name(),
    }
    .to_string();
    RemoteServiceInstallPlan {
        provider: ServiceProviderPlan::new(kind, "ssh-proxy-helper"),
        command,
        action_plan: action_plan.lifecycle_plan,
        reported_service_manager,
    }
}
