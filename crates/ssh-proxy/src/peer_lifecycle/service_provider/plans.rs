use crate::{cli, peer_lifecycle::commands};
use ssh_proxy_core::{
    intent::RemoteInstallIntent,
    model::{PersistenceMode, RemotePlatform},
};

use super::{
    contract::ServiceProviderPlan, kind::ServiceProviderKind, selection::provider_for_platform,
};

#[derive(Debug, Clone)]
pub(crate) struct RemoteServiceInstallPlan {
    pub(crate) provider: ServiceProviderPlan,
    pub(crate) command: String,
    pub(crate) reported_service_manager: String,
}

pub(crate) fn remote_service_install_plan(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> RemoteServiceInstallPlan {
    let intent: RemoteInstallIntent = args.into();
    remote_service_install_plan_for_intent(remote_path, args, &intent)
}

pub(crate) fn remote_service_install_plan_for_intent(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
    intent: &RemoteInstallIntent,
) -> RemoteServiceInstallPlan {
    let kind = provider_for_platform(intent.remote_platform, intent.persistence);
    let command = match intent.persistence {
        PersistenceMode::None => String::new(),
        PersistenceMode::Auto => {
            if intent.remote_platform == RemotePlatform::Windows {
                commands::remote_schtasks_install_command(remote_path, args)
            } else {
                commands::remote_auto_install_command(remote_path, args)
            }
        }
        PersistenceMode::Systemd => commands::remote_systemd_install_command(remote_path, args),
        PersistenceMode::Nohup => commands::remote_nohup_start_command(remote_path, args, true),
        PersistenceMode::Launchd => commands::remote_launchd_install_command(remote_path, args),
        PersistenceMode::Schtasks => commands::remote_schtasks_install_command(remote_path, args),
    };
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
        reported_service_manager,
    }
}
