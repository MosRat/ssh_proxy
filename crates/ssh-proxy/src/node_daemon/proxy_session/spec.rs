use crate::cli;

use ssh_proxy_daemon::{ApplyPolicy, ProxySessionSpec, RemotePortPolicy, SshTargetSpec};

pub(crate) fn proxy_session_spec_from_up_args(args: &cli::UpArgs) -> ProxySessionSpec {
    ProxySessionSpec {
        target: args.target.clone(),
        workspace_id: args.workspace.clone(),
        ssh: ssh_target_spec_from_up_args(args),
        workspace_paths: args.workspace_paths.clone(),
        local_proxy: args.local_proxy.clone(),
        remote_bind: args.remote_bind,
        remote_port_policy: RemotePortPolicy {
            preferred: args.remote_port,
            auto_pick: true,
        },
        connect_mode: args.connect_mode.into(),
        apply_policy: ApplyPolicy {
            vscode_settings: !args.no_remote_machine_settings,
            terminal_env: !args.no_terminal_env,
            server_env: !args.no_server_env,
            git: !args.no_git,
            git_global: !args.no_git_global,
            git_workspace: !args.no_git_workspace,
            git_force_override: !args.no_git_force_override,
            remote_status_file: !args.no_remote_status_file,
            verify_remote_port: !args.no_verify_remote_port,
            no_proxy: args.no_proxy.clone(),
            proxy_support: args.proxy_support.clone(),
            server_dir: args.server_dir.clone(),
        },
    }
}

fn ssh_target_spec_from_up_args(args: &cli::UpArgs) -> Option<SshTargetSpec> {
    let spec = SshTargetSpec {
        host_name: args.ssh_host_name.clone().filter(|value| !value.is_empty()),
        user: args.ssh_user.clone().filter(|value| !value.is_empty()),
        port: args.ssh_port,
        identity: args.ssh_identity.clone(),
        config: args.ssh_config.clone(),
        known_hosts: args.ssh_known_hosts.clone(),
        jump: args.ssh_jump.clone(),
        accept_new: args.ssh_accept_new,
    };
    (!spec.is_empty()).then_some(spec)
}
