use std::{net::IpAddr, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cli;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProxySessionSpec {
    pub(crate) target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) ssh: Option<SshTargetSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) workspace_paths: Vec<String>,
    pub(crate) local_proxy: String,
    pub(crate) remote_bind: IpAddr,
    pub(crate) remote_port_policy: RemotePortPolicy,
    pub(crate) connect_mode: cli::RouteConnectMode,
    pub(crate) apply_policy: ApplyPolicy,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SshTargetSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) host_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) port: Option<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) identity: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) config: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) known_hosts: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) jump: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) accept_new: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RemotePortPolicy {
    pub(crate) preferred: u16,
    pub(crate) auto_pick: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ApplyPolicy {
    pub(crate) vscode_settings: bool,
    pub(crate) terminal_env: bool,
    pub(crate) server_env: bool,
    pub(crate) git: bool,
    pub(crate) git_global: bool,
    pub(crate) git_workspace: bool,
    pub(crate) git_force_override: bool,
    pub(crate) remote_status_file: bool,
    pub(crate) verify_remote_port: bool,
    pub(crate) no_proxy: String,
    pub(crate) proxy_support: String,
    pub(crate) server_dir: String,
}

impl Default for ApplyPolicy {
    fn default() -> Self {
        Self {
            vscode_settings: true,
            terminal_env: true,
            server_env: true,
            git: true,
            git_global: true,
            git_workspace: true,
            git_force_override: true,
            remote_status_file: true,
            verify_remote_port: true,
            no_proxy: "localhost,127.0.0.1,::1".to_string(),
            proxy_support: "override".to_string(),
            server_dir: ".vscode-server".to_string(),
        }
    }
}

impl ProxySessionSpec {
    pub(crate) fn from_up_args(args: &cli::UpArgs) -> Self {
        Self {
            target: args.target.clone(),
            workspace_id: args.workspace.clone(),
            ssh: SshTargetSpec::from_up_args(args),
            workspace_paths: args.workspace_paths.clone(),
            local_proxy: args.local_proxy.clone(),
            remote_bind: args.remote_bind,
            remote_port_policy: RemotePortPolicy {
                preferred: args.remote_port,
                auto_pick: true,
            },
            connect_mode: args.connect_mode,
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

    pub(crate) fn key(&self) -> &str {
        self.workspace_id.as_deref().unwrap_or(&self.target)
    }

    pub(crate) fn route_id(&self) -> String {
        Self::route_id_for_key(self.key())
    }

    pub(crate) fn job_id(&self) -> String {
        Self::job_id_for_key(self.key())
    }

    pub(crate) fn session_id(&self) -> String {
        Self::session_id_for_key(self.key())
    }

    pub(crate) fn route_id_for_key(key: &str) -> String {
        format!("v3-{}", sanitize_key(key))
    }

    pub(crate) fn job_id_for_key(key: &str) -> String {
        format!("proxy:{}", sanitize_key(key))
    }

    pub(crate) fn session_id_for_key(key: &str) -> String {
        format!("session:{}", sanitize_key(key))
    }

    pub(crate) fn remote_url(&self) -> String {
        proxy_url_for_remote(
            &self.local_proxy,
            &self.remote_bind.to_string(),
            self.remote_port_policy.preferred,
        )
    }

    pub(crate) fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| Value::Null)
    }
}

impl SshTargetSpec {
    fn from_up_args(args: &cli::UpArgs) -> Option<Self> {
        let spec = Self {
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

    fn is_empty(&self) -> bool {
        self.host_name.is_none()
            && self.user.is_none()
            && self.port.is_none()
            && self.identity.is_empty()
            && self.config.is_none()
            && self.known_hosts.is_none()
            && self.jump.is_empty()
            && !self.accept_new
    }

    pub(crate) fn ssh_args(&self) -> Vec<String> {
        match self.host_name.as_deref() {
            Some(host_name) => vec!["-o".to_string(), format!("HostName={host_name}")],
            None => Vec::new(),
        }
    }
}

pub(super) fn proxy_url_for_remote(
    local_proxy: &str,
    remote_bind: &str,
    remote_port: u16,
) -> String {
    let Some((scheme, rest)) = local_proxy.split_once("://") else {
        return format!("http://{remote_bind}:{remote_port}");
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let suffix = &rest[authority_end..];
    let userinfo = authority
        .rsplit_once('@')
        .map(|(userinfo, _)| format!("{userinfo}@"))
        .unwrap_or_default();
    format!("{scheme}://{userinfo}{remote_bind}:{remote_port}{suffix}")
}

pub(super) fn sanitize_key(key: &str) -> String {
    let normalized = key
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let trimmed = normalized
        .trim_matches('-')
        .chars()
        .take(64)
        .collect::<String>();
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}
