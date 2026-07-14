use std::collections::BTreeMap;

pub fn build_server_env_setup_content(current: &str, env: &BTreeMap<String, String>) -> String {
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

pub fn build_git_config_script(
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

pub fn build_cleanup_script_with_git(
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

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_script_removes_managed_server_env_block() {
        let script = build_cleanup_script_with_git(
            ".vscode-server",
            &["/home/wen/project".to_string()],
            true,
        );

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
    fn git_config_script_renders_workspace_paths() {
        let script = build_git_config_script(
            "http://127.0.0.1:17890/",
            &["/home/wen/project".to_string()],
            true,
            true,
            true,
        );

        assert!(script.contains("apply_git_pair \"global\" --global"));
        assert!(script.contains("apply_workspace_git '/home/wen/project'"));
        assert!(script.contains("--replace-all"));
    }
}
