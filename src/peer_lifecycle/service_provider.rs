use serde::{Deserialize, Serialize};

use crate::cli;

use super::commands;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ServiceProviderKind {
    WindowsScmSystem,
    WindowsScheduledTaskUser,
    SystemdUser,
    SystemdSystem,
    LaunchdUser,
    LaunchdSystem,
    NohupSupervisor,
}

impl ServiceProviderKind {
    pub(crate) fn manager_name(self) -> &'static str {
        match self {
            Self::WindowsScmSystem => "windows_scm_system",
            Self::WindowsScheduledTaskUser => "windows_schtasks_user",
            Self::SystemdUser => "systemd_user",
            Self::SystemdSystem => "systemd_system",
            Self::LaunchdUser => "launchd_user",
            Self::LaunchdSystem => "launchd_system",
            Self::NohupSupervisor => "nohup_supervisor",
        }
    }

    pub(crate) fn platform(self) -> &'static str {
        match self {
            Self::WindowsScmSystem | Self::WindowsScheduledTaskUser => "windows",
            Self::SystemdUser | Self::SystemdSystem | Self::NohupSupervisor => "linux",
            Self::LaunchdUser | Self::LaunchdSystem => "macos",
        }
    }

    pub(crate) fn persistent(self) -> bool {
        !matches!(self, Self::NohupSupervisor)
    }

    pub(crate) fn requires_elevation(self) -> bool {
        matches!(
            self,
            Self::WindowsScmSystem | Self::SystemdSystem | Self::LaunchdSystem
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServiceProviderPlan {
    pub(crate) kind: ServiceProviderKind,
    pub(crate) service_name: String,
    pub(crate) install_hint: String,
    pub(crate) status_hint: String,
}

impl ServiceProviderPlan {
    pub(crate) fn new(kind: ServiceProviderKind, service_name: impl Into<String>) -> Self {
        let service_name = service_name.into();
        let install_hint = match kind {
            ServiceProviderKind::WindowsScmSystem => {
                "windows-service via elevated install-worker".to_string()
            }
            ServiceProviderKind::WindowsScheduledTaskUser => {
                "schtasks /Create /SC ONLOGON".to_string()
            }
            ServiceProviderKind::SystemdUser => "systemctl --user enable --now".to_string(),
            ServiceProviderKind::SystemdSystem => "systemctl enable --now".to_string(),
            ServiceProviderKind::LaunchdUser => "launchctl bootstrap gui/<uid>".to_string(),
            ServiceProviderKind::LaunchdSystem => "launchctl bootstrap system".to_string(),
            ServiceProviderKind::NohupSupervisor => "managed nohup supervisor script".to_string(),
        };
        let status_hint = match kind {
            ServiceProviderKind::WindowsScmSystem => "sc.exe query".to_string(),
            ServiceProviderKind::WindowsScheduledTaskUser => "schtasks /Query".to_string(),
            ServiceProviderKind::SystemdUser => "systemctl --user status".to_string(),
            ServiceProviderKind::SystemdSystem => "systemctl status".to_string(),
            ServiceProviderKind::LaunchdUser => "launchctl print gui/<uid>".to_string(),
            ServiceProviderKind::LaunchdSystem => "launchctl print system".to_string(),
            ServiceProviderKind::NohupSupervisor => "pidfile plus child process probe".to_string(),
        };
        Self {
            kind,
            service_name,
            install_hint,
            status_hint,
        }
    }
}

pub(crate) fn provider_for_remote_os(
    remote_os: cli::RemoteOs,
    persist: cli::PersistMode,
) -> ServiceProviderKind {
    match persist {
        cli::PersistMode::Systemd => ServiceProviderKind::SystemdUser,
        cli::PersistMode::Nohup => ServiceProviderKind::NohupSupervisor,
        cli::PersistMode::Launchd => ServiceProviderKind::LaunchdUser,
        cli::PersistMode::Schtasks => ServiceProviderKind::WindowsScheduledTaskUser,
        cli::PersistMode::None => ServiceProviderKind::NohupSupervisor,
        cli::PersistMode::Auto => match remote_os {
            cli::RemoteOs::Windows => ServiceProviderKind::WindowsScheduledTaskUser,
            cli::RemoteOs::Unix | cli::RemoteOs::Auto => ServiceProviderKind::SystemdUser,
        },
    }
}

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
        cli::PersistMode::Schtasks => {
            commands::remote_schtasks_install_command(remote_path, args)
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_reports_stable_manager_names() {
        assert_eq!(
            ServiceProviderKind::WindowsScmSystem.manager_name(),
            "windows_scm_system"
        );
        assert_eq!(
            ServiceProviderKind::SystemdUser.manager_name(),
            "systemd_user"
        );
        assert!(ServiceProviderKind::WindowsScmSystem.requires_elevation());
        assert!(!ServiceProviderKind::WindowsScheduledTaskUser.requires_elevation());
    }

    #[test]
    fn remote_provider_defaults_match_production_order() {
        assert_eq!(
            provider_for_remote_os(cli::RemoteOs::Windows, cli::PersistMode::Auto),
            ServiceProviderKind::WindowsScheduledTaskUser
        );
        assert_eq!(
            provider_for_remote_os(cli::RemoteOs::Auto, cli::PersistMode::Auto),
            ServiceProviderKind::SystemdUser
        );
    }

    #[test]
    fn remote_install_plan_preserves_auto_reporting_contract() {
        let args = cli::InstallRemoteArgs {
            target: "edge".to_string(),
            ssh_args: Vec::new(),
            ssh_command: None,
            user: None,
            port: None,
            identity: Vec::new(),
            config: None,
            known_hosts: None,
            accept_new: false,
            insecure_ignore_host_key: false,
            jump: Vec::new(),
            remote_path: None,
            remote_bin: None,
            remote_os: cli::RemoteOs::Unix,
            remote_token: Some("secret".to_string()),
            remote_tcp: "127.0.0.1:19080".parse().unwrap(),
            remote_control: "127.0.0.1:19081".parse().unwrap(),
            local_node_id: None,
            local_node_name: None,
            local_control_endpoint: None,
            local_transport: None,
            remote_node_id: None,
            remote_node_name: None,
            remote_tls_transport: None,
            remote_quic_transport: None,
            remote_tls_cert: None,
            remote_tls_key: None,
            remote_tls_client_ca: None,
            persist: cli::PersistMode::Auto,
        };

        let plan = remote_service_install_plan("/home/me/bin/ssh_proxy", &args);

        assert_eq!(plan.provider.kind, ServiceProviderKind::SystemdUser);
        assert_eq!(plan.reported_service_manager, "auto");
        assert!(plan.command.contains("systemctl --user"));
        assert!(plan.command.contains("nohup"));
    }
}
