use crate::{cli, peer_lifecycle::commands};

use super::{
    contract::ServiceProviderPlan, kind::ServiceProviderKind, selection::provider_for_remote_os,
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
    let kind = provider_for_remote_os(args.remote_os, args.persist);
    let command = match args.persist {
        cli::PersistMode::None => String::new(),
        cli::PersistMode::Auto => {
            if args.remote_os == cli::RemoteOs::Windows {
                commands::remote_schtasks_install_command(remote_path, args)
            } else {
                commands::remote_auto_install_command(remote_path, args)
            }
        }
        cli::PersistMode::Systemd => commands::remote_systemd_install_command(remote_path, args),
        cli::PersistMode::Nohup => commands::remote_nohup_start_command(remote_path, args, true),
        cli::PersistMode::Launchd => commands::remote_launchd_install_command(remote_path, args),
        cli::PersistMode::Schtasks => commands::remote_schtasks_install_command(remote_path, args),
    };
    let reported_service_manager = match args.persist {
        cli::PersistMode::None => "none",
        cli::PersistMode::Auto if args.remote_os == cli::RemoteOs::Windows => {
            ServiceProviderKind::WindowsScheduledTaskUser.manager_name()
        }
        cli::PersistMode::Auto => "auto",
        _ => kind.manager_name(),
    }
    .to_string();
    RemoteServiceInstallPlan {
        provider: ServiceProviderPlan::new(kind, "ssh-proxy-helper"),
        command,
        reported_service_manager,
    }
}
