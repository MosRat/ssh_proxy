use std::{net::IpAddr, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ssh_proxy_core::model::RouteConnectMode;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxySessionSpec {
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<SshTargetSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_paths: Vec<String>,
    pub local_proxy: String,
    pub remote_bind: IpAddr,
    pub remote_port_policy: RemotePortPolicy,
    pub connect_mode: RouteConnectMode,
    pub apply_policy: ApplyPolicy,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SshTargetSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identity: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_hosts: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jump: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub accept_new: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotePortPolicy {
    pub preferred: u16,
    pub auto_pick: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApplyPolicy {
    pub vscode_settings: bool,
    pub terminal_env: bool,
    pub server_env: bool,
    pub git: bool,
    pub git_global: bool,
    pub git_workspace: bool,
    pub git_force_override: bool,
    pub remote_status_file: bool,
    pub verify_remote_port: bool,
    pub no_proxy: String,
    pub proxy_support: String,
    pub server_dir: String,
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
    pub fn key(&self) -> &str {
        self.workspace_id.as_deref().unwrap_or(&self.target)
    }

    pub fn route_id(&self) -> String {
        Self::route_id_for_key(self.key())
    }

    pub fn job_id(&self) -> String {
        Self::job_id_for_key(self.key())
    }

    pub fn session_id(&self) -> String {
        Self::session_id_for_key(self.key())
    }

    pub fn route_id_for_key(key: &str) -> String {
        format!("v3-{}", sanitize_key(key))
    }

    pub fn job_id_for_key(key: &str) -> String {
        format!("proxy:{}", sanitize_key(key))
    }

    pub fn session_id_for_key(key: &str) -> String {
        format!("session:{}", sanitize_key(key))
    }

    pub fn remote_url(&self) -> String {
        proxy_url_for_remote(
            &self.local_proxy,
            &self.remote_bind.to_string(),
            self.remote_port_policy.preferred,
        )
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| Value::Null)
    }
}

impl SshTargetSpec {
    pub fn is_empty(&self) -> bool {
        self.host_name.is_none()
            && self.user.is_none()
            && self.port.is_none()
            && self.identity.is_empty()
            && self.config.is_none()
            && self.known_hosts.is_none()
            && self.jump.is_empty()
            && !self.accept_new
    }

    pub fn ssh_args(&self) -> Vec<String> {
        match self.host_name.as_deref() {
            Some(host_name) => vec!["-o".to_string(), format!("HostName={host_name}")],
            None => Vec::new(),
        }
    }
}

pub fn proxy_session_specs_match(left: &ProxySessionSpec, right: &ProxySessionSpec) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    normalize_proxy_session_spec_for_live_reuse(&mut left);
    normalize_proxy_session_spec_for_live_reuse(&mut right);
    serde_json::to_value(left).ok() == serde_json::to_value(right).ok()
}

pub fn normalize_proxy_session_spec_for_live_reuse(spec: &mut ProxySessionSpec) {
    if let Some(ssh) = spec.ssh.as_mut() {
        ssh.identity.clear();
    }
}

pub fn proxy_url_for_remote(local_proxy: &str, remote_bind: &str, remote_port: u16) -> String {
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

pub fn sanitize_key(key: &str) -> String {
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn spec() -> ProxySessionSpec {
        ProxySessionSpec {
            target: "126".to_string(),
            workspace_id: Some("Window A".to_string()),
            ssh: None,
            workspace_paths: Vec::new(),
            local_proxy: "http://127.0.0.1:10808/".to_string(),
            remote_bind: "127.0.0.1".parse::<IpAddr>().unwrap(),
            remote_port_policy: RemotePortPolicy {
                preferred: 17890,
                auto_pick: true,
            },
            connect_mode: RouteConnectMode::ReverseLink,
            apply_policy: ApplyPolicy::default(),
        }
    }

    #[test]
    fn proxy_session_spec_derives_stable_ids() {
        let spec = spec();

        assert_eq!(spec.route_id(), "v3-window-a");
        assert_eq!(spec.job_id(), "proxy:window-a");
        assert_eq!(spec.session_id(), "session:window-a");
        assert_eq!(spec.remote_url(), "http://127.0.0.1:17890/");
    }

    #[test]
    fn proxy_url_preserves_userinfo_and_suffix() {
        assert_eq!(
            proxy_url_for_remote("http://user:pass@127.0.0.1:10808/path", "127.0.0.1", 17890),
            "http://user:pass@127.0.0.1:17890/path",
        );
    }

    #[test]
    fn ssh_target_spec_renders_host_override_args() {
        let ssh = SshTargetSpec {
            host_name: Some("10.10.100.71".to_string()),
            ..SshTargetSpec::default()
        };

        assert_eq!(ssh.ssh_args(), vec!["-o", "HostName=10.10.100.71"]);
    }

    #[test]
    fn proxy_session_reuse_ignores_identity_enrichment() {
        let mut existing = ProxySessionSpec {
            target: "125".to_string(),
            workspace_id: Some("wenhongli@172.18.116.125".to_string()),
            ssh: Some(SshTargetSpec {
                host_name: Some("172.18.116.125".to_string()),
                user: Some("wenhongli".to_string()),
                port: None,
                identity: Vec::new(),
                config: Some(PathBuf::from("C:/Users/whl/.ssh/config")),
                known_hosts: Some(PathBuf::from("C:/Users/whl/.ssh/known_hosts")),
                jump: Vec::new(),
                accept_new: true,
            }),
            workspace_paths: Vec::new(),
            local_proxy: "http://127.0.0.1:10808/".to_string(),
            remote_bind: "127.0.0.1".parse::<IpAddr>().unwrap(),
            remote_port_policy: RemotePortPolicy {
                preferred: 17890,
                auto_pick: true,
            },
            connect_mode: RouteConnectMode::ReverseLink,
            apply_policy: ApplyPolicy::default(),
        };
        let mut enriched = existing.clone();
        enriched.ssh.as_mut().unwrap().identity = vec![
            PathBuf::from("C:/Users/whl/.ssh/id_rsa"),
            PathBuf::from("C:/Users/whl/.ssh/id_ed25519"),
        ];

        assert!(proxy_session_specs_match(&existing, &enriched));

        existing.remote_port_policy.preferred = 17891;
        assert!(!proxy_session_specs_match(&existing, &enriched));
    }
}
