use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::{peer_lifecycle::artifacts::PeerArtifact, ssh_client};

#[cfg(test)]
use super::proxy_session::ProxySessionSpec;

mod artifacts;
mod executor;
mod payload;
mod shell;

use artifacts::{read_remote_setup_artifact, write_remote_setup_artifact};
pub(super) use executor::{apply_remote_settings, cleanup_remote_settings};
#[cfg(test)]
use payload::{setup_hash, setup_payload};
use shell::shell_quote;

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

fn build_server_env_setup_content(current: &str, env: &BTreeMap<String, String>) -> String {
    let lines = env
        .iter()
        .map(|(key, value)| format!("export {}={}", key, shell_quote(value)))
        .collect::<Vec<_>>();
    let block = [
        "# >>> vscode-remote-proxy >>>".to_string(),
        lines.join("\n"),
        "# <<< vscode-remote-proxy <<<".to_string(),
    ]
    .join("\n");
    let mut cleaned = strip_managed_server_env_block(current);
    if !cleaned.is_empty() && !cleaned.ends_with('\n') {
        cleaned.push('\n');
    }
    cleaned.push_str(&block);
    cleaned.push('\n');
    cleaned
}

fn strip_managed_server_env_block(current: &str) -> String {
    let mut cleaned = Vec::new();
    let mut skip = false;
    for line in current.lines() {
        match line.trim() {
            "# >>> vscode-remote-proxy >>>" => {
                skip = true;
                continue;
            }
            "# <<< vscode-remote-proxy <<<" => {
                skip = false;
                continue;
            }
            _ if skip => continue,
            _ => cleaned.push(line),
        }
    }
    cleaned.join("\n").trim_end_matches('\n').to_string()
}

fn build_git_config_script(
    proxy_url: &str,
    workspace_paths: &[String],
    apply_global: bool,
    apply_workspace: bool,
    force_override: bool,
) -> String {
    let replace_arg = if force_override { "--replace-all" } else { "" };
    let workspace_lines = workspace_paths
        .iter()
        .map(|workspace_path| format!("apply_workspace_git {}", shell_quote(workspace_path)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"
set -u
proxy_url={proxy_url}
replace_arg={replace_arg}

if ! command -v git >/dev/null 2>&1; then
  echo "remote-proxy: git not found on remote; skipped Git proxy config"
  exit 0
fi

apply_git_pair() {{
  scope_label="$1"
  shift
  config_scope="$1"
  shift
  if [ -n "$replace_arg" ]; then
    if git "$@" config "$config_scope" "$replace_arg" http.proxy "$proxy_url" && git "$@" config "$config_scope" "$replace_arg" https.proxy "$proxy_url"; then
      echo "remote-proxy: patched Git proxy for $scope_label"
      return 0
    fi
  elif git "$@" config "$config_scope" http.proxy "$proxy_url" && git "$@" config "$config_scope" https.proxy "$proxy_url"; then
    echo "remote-proxy: patched Git proxy for $scope_label"
    return 0
  fi
  echo "remote-proxy: failed to patch Git proxy for $scope_label" >&2
  return 1
}}

apply_workspace_git() {{
  workspace_path="$1"
  if [ ! -d "$workspace_path" ]; then
    echo "remote-proxy: workspace path missing, skipped Git proxy: $workspace_path"
    return 0
  fi
  if ! git -C "$workspace_path" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "remote-proxy: workspace is not a Git worktree, skipped: $workspace_path"
    return 0
  fi
  top_level="$(git -C "$workspace_path" rev-parse --show-toplevel 2>/dev/null || printf '%s' "$workspace_path")"
  apply_git_pair "workspace:$top_level" --local -C "$top_level"
}}

{global_line}

if {apply_workspace}; then
  if [ {workspace_count} -eq 0 ]; then
    echo "remote-proxy: no remote workspace folders available for workspace Git proxy config"
  else
{workspace_lines}
  fi
else
  echo "remote-proxy: workspace Git proxy config disabled"
fi
"#,
        proxy_url = shell_quote(proxy_url),
        replace_arg = shell_quote(replace_arg),
        global_line = if apply_global {
            r#"apply_git_pair "global" --global || true"#
        } else {
            r#"echo "remote-proxy: global Git proxy config disabled""#
        },
        apply_workspace = if apply_workspace { "true" } else { "false" },
        workspace_count = workspace_paths.len(),
        workspace_lines = if workspace_lines.is_empty() {
            "    :".to_string()
        } else {
            workspace_lines
        },
    )
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
fn build_cleanup_script(server_dir: &str, workspace_paths: &[String]) -> String {
    build_cleanup_script_with_git(server_dir, workspace_paths, true)
}

fn build_cleanup_script_with_git(
    server_dir: &str,
    workspace_paths: &[String],
    include_git: bool,
) -> String {
    let workspace_lines = workspace_paths
        .iter()
        .map(|workspace_path| format!("cleanup_workspace_git {}", shell_quote(workspace_path)))
        .collect::<Vec<_>>()
        .join("\n");
    let git_cleanup = if include_git {
        format!(
            r#"
if command -v git >/dev/null 2>&1; then
  git config --global --unset-all http.proxy >/dev/null 2>&1 || true
  git config --global --unset-all https.proxy >/dev/null 2>&1 || true
  echo "remote-proxy: cleaned global Git proxy config"

  cleanup_workspace_git() {{
    workspace_path="$1"
    if [ ! -d "$workspace_path" ]; then
      echo "remote-proxy: workspace path missing, skipped Git cleanup: $workspace_path"
      return 0
    fi
    if ! git -C "$workspace_path" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
      echo "remote-proxy: workspace is not a Git worktree, skipped cleanup: $workspace_path"
      return 0
    fi
    top_level="$(git -C "$workspace_path" rev-parse --show-toplevel 2>/dev/null || printf '%s' "$workspace_path")"
    git -C "$top_level" config --local --unset-all http.proxy >/dev/null 2>&1 || true
    git -C "$top_level" config --local --unset-all https.proxy >/dev/null 2>&1 || true
    echo "remote-proxy: cleaned workspace Git proxy config: $top_level"
  }}

  if [ {workspace_count} -eq 0 ]; then
    echo "remote-proxy: no remote workspace folders available for workspace Git cleanup"
  else
{workspace_lines}
  fi
else
  echo "remote-proxy: git not found on remote; skipped Git proxy cleanup"
fi
"#,
            workspace_count = workspace_paths.len(),
            workspace_lines = if workspace_lines.is_empty() {
                "    :".to_string()
            } else {
                workspace_lines
            },
        )
    } else {
        r#"echo "remote-proxy: Git proxy cleanup handled by ssh_proxy remote admin""#.to_string()
    };
    format!(
        r#"
set -eu
server_dir={shell_server_dir}

{git_cleanup}

status_file="$HOME/$server_dir/remote-proxy-status.json"
rm -f "$status_file"

env_file="$HOME/$server_dir/server-env-setup"
if [ -f "$env_file" ]; then
  tmp="$env_file.tmp.$$"
  awk '
    /# >>> vscode-remote-proxy >>>/ {{ skip=1; next }}
    /# <<< vscode-remote-proxy <<</ {{ skip=0; next }}
    skip != 1 {{ print }}
  ' "$env_file" > "$tmp"
  chmod 600 "$tmp"
  mv "$tmp" "$env_file"
fi

echo "remote-proxy: cleanup complete"
"#,
        shell_server_dir = shell_quote(server_dir),
        git_cleanup = git_cleanup,
    )
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
            remote_port_policy: super::super::proxy_session::RemotePortPolicy {
                preferred: 17890,
                auto_pick: true,
            },
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
    fn cleanup_script_removes_managed_server_env_block() {
        let script = build_cleanup_script(".vscode-server", &["/home/wen/project".to_string()]);

        assert!(script.contains("remote-proxy-status.json"));
        assert!(script.contains("# >>> vscode-remote-proxy >>>"));
        assert!(script.contains("cleanup_workspace_git '/home/wen/project'"));
    }

    #[test]
    fn server_env_setup_content_replaces_existing_managed_block() {
        let mut env = BTreeMap::new();
        env.insert(
            "HTTPS_PROXY".to_string(),
            "http://127.0.0.1:17890/".to_string(),
        );
        let current = "export KEEP=1\n# >>> vscode-remote-proxy >>>\nexport OLD=1\n# <<< vscode-remote-proxy <<<\nexport AFTER=1\n";

        let content = build_server_env_setup_content(current, &env);

        assert!(content.contains("export KEEP=1"));
        assert!(content.contains("export AFTER=1"));
        assert!(content.contains("export HTTPS_PROXY='http://127.0.0.1:17890/'"));
        assert!(!content.contains("export OLD=1"));
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
