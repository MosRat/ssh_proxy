use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use serde_json::Value;
use ssh_proxy_deploy::build_server_env_setup_content;

use crate::{peer_lifecycle::artifacts::PeerArtifact, ssh_client};

#[cfg(test)]
use super::proxy_session::ProxySessionSpec;

mod artifacts;
mod executor;
mod payload;

use artifacts::{read_remote_setup_artifact, write_remote_setup_artifact};
pub(super) use executor::{apply_remote_settings, cleanup_remote_settings};
#[cfg(test)]
use payload::{setup_hash, setup_payload};

async fn apply_remote_machine_settings(client: &ssh_client::Client, payload: &Value) -> Result<()> {
    let server_dir = payload
        .get("server_dir")
        .and_then(Value::as_str)
        .unwrap_or(".vscode-server");
    let current = read_remote_machine_settings(client, server_dir).await?;
    let mut settings = parse_settings_object(&current);
    let values = payload
        .get("values")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("remote settings payload is missing values"))?;
    for (key, value) in values {
        if key.starts_with("terminal.integrated.env.") {
            let mut merged = settings
                .get(key)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(env) = value.as_object() {
                for (env_key, env_value) in env {
                    merged.insert(env_key.clone(), env_value.clone());
                }
            }
            settings.insert(key.clone(), Value::Object(merged));
        } else {
            settings.insert(key.clone(), value.clone());
        }
    }
    write_remote_machine_settings(client, server_dir, &Value::Object(settings)).await
}

async fn cleanup_remote_machine_settings(
    client: &ssh_client::Client,
    server_dir: &str,
    env_keys: &[&str],
) -> Result<()> {
    let current = read_remote_machine_settings(client, server_dir).await?;
    if current.trim().is_empty() {
        return Ok(());
    }
    let mut settings = parse_settings_object(&current);
    settings.remove("http.proxy");
    settings.remove("http.proxySupport");
    for key in [
        "terminal.integrated.env.linux",
        "terminal.integrated.env.osx",
        "terminal.integrated.env.windows",
    ] {
        if let Some(Value::Object(mut env)) = settings.remove(key) {
            for env_key in env_keys {
                env.remove(*env_key);
            }
            if !env.is_empty() {
                settings.insert(key.to_string(), Value::Object(env));
            }
        }
    }
    write_remote_machine_settings(client, server_dir, &Value::Object(settings)).await
}

async fn read_remote_machine_settings(
    client: &ssh_client::Client,
    server_dir: &str,
) -> Result<String> {
    read_remote_setup_artifact(
        client,
        server_dir,
        "data/Machine/settings.json",
        "read remote VS Code machine settings",
    )
    .await
}

async fn write_remote_machine_settings(
    client: &ssh_client::Client,
    server_dir: &str,
    settings: &Value,
) -> Result<()> {
    let text = serde_json::to_string_pretty(settings)? + "\n";
    write_remote_setup_artifact(
        client,
        server_dir,
        "data/Machine/settings.json",
        PeerArtifact::VscodeMachineSettings,
        text.into_bytes(),
        true,
        "write remote VS Code machine settings",
    )
    .await
}

fn parse_settings_object(raw: &str) -> serde_json::Map<String, Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return serde_json::Map::new();
    }
    serde_json::from_str::<Value>(trimmed)
        .or_else(|_| {
            serde_json::from_str::<Value>(&strip_json_comments_and_trailing_commas(trimmed))
        })
        .ok()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

fn strip_json_comments_and_trailing_commas(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if in_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            output.push(ch);
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            for next in chars.by_ref() {
                if next == '\n' {
                    output.push('\n');
                    break;
                }
            }
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut previous = '\0';
            for next in chars.by_ref() {
                if previous == '*' && next == '/' {
                    break;
                }
                previous = next;
            }
            continue;
        }
        output.push(ch);
    }
    remove_trailing_json_commas(&output)
}

fn remove_trailing_json_commas(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if in_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            output.push(ch);
            continue;
        }
        if ch == ',' {
            let mut lookahead = chars.clone();
            while matches!(lookahead.peek(), Some(next) if next.is_whitespace()) {
                lookahead.next();
            }
            if matches!(lookahead.peek(), Some('}' | ']')) {
                continue;
            }
        }
        output.push(ch);
    }
    output
}

async fn write_remote_server_env_setup(
    client: &ssh_client::Client,
    server_dir: &str,
    env: &BTreeMap<String, String>,
) -> Result<()> {
    let current = read_remote_setup_artifact(
        client,
        server_dir,
        "server-env-setup",
        "read server-env-setup",
    )
    .await?;
    let content = build_server_env_setup_content(&current, env);
    write_remote_setup_artifact(
        client,
        server_dir,
        "server-env-setup",
        PeerArtifact::VscodeServerEnv,
        content.into_bytes(),
        true,
        "patch remote server-env-setup",
    )
    .await
}

async fn write_remote_status_file(
    client: &ssh_client::Client,
    server_dir: &str,
    payload: &Value,
) -> Result<()> {
    let mut bytes = serde_json::to_vec(payload)?;
    bytes.push(b'\n');
    write_remote_setup_artifact(
        client,
        server_dir,
        "remote-proxy-status.json",
        PeerArtifact::VscodeRemoteStatus,
        bytes,
        false,
        "write remote proxy status file",
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use ssh_proxy_core::model::RouteConnectMode;

    use super::*;
    use ssh_proxy_deploy::{RemoteArtifactIntent, RemoteArtifactKind};

    fn spec() -> ProxySessionSpec {
        ProxySessionSpec {
            target: "126".to_string(),
            workspace_id: Some("Window A".to_string()),
            ssh: None,
            workspace_paths: vec!["/home/wen/project".to_string()],
            local_proxy: "http://127.0.0.1:10808/".to_string(),
            remote_bind: "127.0.0.1".parse::<IpAddr>().unwrap(),
            remote_port_policy: super::super::proxy_session::RemotePortPolicy::new(17890),
            connect_mode: RouteConnectMode::ReverseLink,
            apply_policy: super::super::proxy_session::ApplyPolicy::default(),
        }
    }

    #[test]
    fn setup_payload_uses_existing_status_file_contract() {
        let spec = spec();
        let payload = setup_payload(&spec, "http://127.0.0.1:17890/", None);

        assert_eq!(payload["proxyUrl"], "http://127.0.0.1:17890/");
        assert_eq!(payload["bindHost"], "127.0.0.1");
        assert_eq!(payload["routeId"], "v3-window-a");
        assert_eq!(payload["workspacePaths"][0], "/home/wen/project");
        assert_eq!(payload["values"]["http.proxy"], "http://127.0.0.1:17890/");
    }

    #[test]
    fn remote_setup_hash_is_stable_for_same_payload() {
        let spec = spec();
        let left = setup_hash(&setup_payload(&spec, "http://127.0.0.1:17890/", None));
        let right = setup_hash(&setup_payload(&spec, "http://127.0.0.1:17890/", None));

        assert_eq!(left, right);
        assert_eq!(left.len(), 64);
    }

    #[test]
    fn remote_setup_artifact_commands_use_stdin_without_content_embedding() {
        let write = RemoteArtifactIntent::new(
            ".vscode-server",
            "data/Machine/settings.json",
            RemoteArtifactKind::VscodeMachineSettings,
            true,
            "write settings",
        )
        .write_command();
        let read = RemoteArtifactIntent::new(
            ".vscode-server",
            "remote-proxy-status.json",
            RemoteArtifactKind::VscodeRemoteStatus,
            false,
            "read status",
        )
        .read_command();

        assert!(write.contains("cat > \"$tmp\""));
        assert!(write.contains(".vscode-remote-proxy.bak"));
        assert!(!write.contains("<<'JSON'"));
        assert!(read.contains("cat \"$target\""));
    }

    #[test]
    fn remote_setup_does_not_embed_shell_port_verification() {
        let setup_source = include_str!("remote_setup.rs");

        assert!(!setup_source.contains(&["nc", " -z"].concat()));
        assert!(!setup_source.contains(&["/dev", "/tcp"].concat()));
        assert!(!setup_source.contains(&["socket", ".create_connection"].concat()));
        assert!(!setup_source.contains(&["command -v ", "node"].concat()));
        assert!(!setup_source.contains(&["node", "_bin"].concat()));
    }
}
