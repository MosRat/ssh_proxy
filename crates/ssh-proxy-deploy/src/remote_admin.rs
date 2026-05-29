use std::{net::SocketAddr, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ssh_proxy_core::model::RemotePlatform;

use crate::commands::sh_quote;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum RemoteAdminIntent {
    Checksum {
        path: String,
    },
    Defaults {
        preferred_transport: SocketAddr,
        preferred_control: SocketAddr,
    },
    Status {
        remote_tcp: Option<SocketAddr>,
        remote_path: Option<String>,
    },
    Doctor {
        remote_tcp: Option<SocketAddr>,
        remote_path: Option<String>,
    },
    GitApply {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        config_path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_path: Option<String>,
        http_proxy: Option<String>,
        https_proxy: Option<String>,
    },
    GitCleanup {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        config_path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_path: Option<String>,
    },
}

impl RemoteAdminIntent {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Checksum { .. } => "remote_admin_checksum",
            Self::Defaults { .. } => "remote_admin_defaults",
            Self::Status { .. } => "remote_admin_status",
            Self::Doctor { .. } => "remote_admin_doctor",
            Self::GitApply { .. } => "remote_admin_git_apply",
            Self::GitCleanup { .. } => "remote_admin_git_cleanup",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteAdminChecksumReport {
    pub path: String,
    pub sha256: String,
    pub len: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteAdminDefaultsReport {
    pub transport: SocketAddr,
    pub control: SocketAddr,
    pub node_id: Option<String>,
    pub node_name: String,
    pub config: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteAdminGitConfigReport {
    pub config_path: String,
    pub changed: bool,
    pub removed_values: usize,
}

pub fn remote_admin_stdin_command(remote_path: &str, remote_platform: RemotePlatform) -> String {
    match remote_platform {
        RemotePlatform::Windows => format!("{} remote admin", windows_command_arg(remote_path)),
        RemotePlatform::Unix | RemotePlatform::Auto => {
            format!("{} remote admin", sh_quote(remote_path))
        }
    }
}

pub fn remote_admin_ok(kind: &str, data: Value) -> Value {
    json!({
        "ok": true,
        "kind": kind,
        "execution_backend": "own_binary",
        "native_api_available": true,
        "fallback_used": false,
        "data": data,
    })
}

pub fn remote_admin_error(kind: &str, error: &str) -> Value {
    json!({
        "ok": false,
        "kind": kind,
        "execution_backend": "own_binary",
        "native_api_available": true,
        "fallback_used": false,
        "error": error,
    })
}

pub fn apply_git_proxy_config(
    config_path: &Path,
    http_proxy: Option<&str>,
    https_proxy: Option<&str>,
) -> Result<RemoteAdminGitConfigReport, String> {
    let before = std::fs::read(config_path).ok();
    let mut config = load_git_config(config_path)?;
    if let Some(proxy) = http_proxy {
        config
            .set_raw_value(&"http.proxy", proxy)
            .map_err(|err| err.to_string())?;
    }
    if let Some(proxy) = https_proxy {
        config
            .set_raw_value(&"https.proxy", proxy)
            .map_err(|err| err.to_string())?;
    }
    let after = write_git_config(config_path, &config)?;
    Ok(RemoteAdminGitConfigReport {
        config_path: config_path.display().to_string(),
        changed: before.as_deref() != Some(after.as_slice()),
        removed_values: 0,
    })
}

pub fn cleanup_git_proxy_config(config_path: &Path) -> Result<RemoteAdminGitConfigReport, String> {
    let before = std::fs::read(config_path).ok();
    let mut config = load_git_config(config_path)?;
    let mut removed_values = 0;
    for section_name in ["http", "https"] {
        if let Ok(mut section) = config.section_mut(section_name, None) {
            while section.remove("proxy").is_some() {
                removed_values += 1;
            }
        }
    }
    let after = write_git_config(config_path, &config)?;
    Ok(RemoteAdminGitConfigReport {
        config_path: config_path.display().to_string(),
        changed: before.as_deref() != Some(after.as_slice()),
        removed_values,
    })
}

fn load_git_config(path: &Path) -> Result<gix_config::File<'static>, String> {
    if !path.exists() {
        return Ok(gix_config::File::default());
    }
    gix_config::File::from_path_no_includes(path.to_path_buf(), gix_config::Source::User)
        .map_err(|err| err.to_string())
}

fn write_git_config(path: &Path, config: &gix_config::File<'_>) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    config.write_to(&mut bytes).map_err(|err| err.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(path, &bytes).map_err(|err| err.to_string())?;
    Ok(bytes)
}

fn windows_command_arg(value: &str) -> String {
    if value.starts_with('"') && value.ends_with('"') {
        return value.to_string();
    }
    format!("\"{}\"", value.replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_admin_intent_uses_stable_command_tag() {
        let intent = RemoteAdminIntent::Checksum {
            path: "/tmp/ssh_proxy".to_string(),
        };
        let value = serde_json::to_value(&intent).unwrap();

        assert_eq!(value["command"], "checksum");
        assert_eq!(intent.kind(), "remote_admin_checksum");
    }

    #[test]
    fn remote_admin_stdin_command_quotes_unix_path() {
        let command = remote_admin_stdin_command("/tmp/ssh proxy", RemotePlatform::Unix);

        assert_eq!(command, "'/tmp/ssh proxy' remote admin");
        assert!(!command.contains(" sha256sum "));
    }

    #[test]
    fn git_cleanup_removes_proxy_values() {
        let dir =
            std::env::temp_dir().join(format!("ssh-proxy-git-config-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config");
        std::fs::write(
            &path,
            "[http]\n\tproxy = http://127.0.0.1:1\n[https]\n\tproxy = http://127.0.0.1:1\n",
        )
        .unwrap();

        let report = cleanup_git_proxy_config(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();

        assert!(report.changed);
        assert_eq!(report.removed_values, 2);
        assert!(!text.contains("proxy ="));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
