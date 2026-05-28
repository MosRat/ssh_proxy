use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{config, ssh_client};

use super::{proxy_session::ProxySessionSpec, remote_ssh};

#[derive(Debug, Clone, Serialize)]
pub(super) struct RemoteSetupOutcome {
    pub(super) desired_hash: String,
    pub(super) applied_hash: String,
    pub(super) remote_url: String,
    pub(super) verified: bool,
}

pub(super) async fn apply_remote_settings(
    config: &config::AppConfig,
    spec: &ProxySessionSpec,
    route: Option<&Value>,
    remote_url: &str,
) -> Result<RemoteSetupOutcome> {
    let install_args = remote_ssh::install_args_from_spec(config, spec)
        .context("failed to build SSH target for remote setup")?;
    let client = ssh_client::Client::connect_install_args(&install_args).await?;
    let payload = setup_payload(spec, remote_url, route);
    let desired_hash = setup_hash(&payload);

    if spec.apply_policy.vscode_settings {
        run_script(
            &client,
            &build_remote_settings_script(&payload)?,
            "patch remote VS Code machine settings",
        )
        .await?;
    }

    if spec.apply_policy.server_env {
        run_script(
            &client,
            &build_server_env_setup_script(
                &spec.apply_policy.server_dir,
                &build_proxy_env(remote_url, &spec.apply_policy.no_proxy),
            ),
            "patch remote server-env-setup",
        )
        .await?;
    }

    if spec.apply_policy.git {
        if spec.apply_policy.git_global || spec.apply_policy.git_workspace {
            run_script(
                &client,
                &build_git_config_script(
                    remote_url,
                    &spec.workspace_paths,
                    spec.apply_policy.git_global,
                    spec.apply_policy.git_workspace,
                    spec.apply_policy.git_force_override,
                ),
                "apply remote Git proxy config",
            )
            .await?;
        }
    }

    if spec.apply_policy.remote_status_file {
        run_script(
            &client,
            &build_remote_status_file_script(&spec.apply_policy.server_dir, &payload)?,
            "write remote proxy status file",
        )
        .await?;
    }

    Ok(RemoteSetupOutcome {
        desired_hash: desired_hash.clone(),
        applied_hash: desired_hash,
        remote_url: remote_url.to_string(),
        verified: false,
    })
}

pub(super) async fn cleanup_remote_settings(
    config: &config::AppConfig,
    spec: &ProxySessionSpec,
    remote_url: &str,
) -> Result<()> {
    let install_args = remote_ssh::install_args_from_spec(config, spec)
        .context("failed to build SSH target for remote cleanup")?;
    let client = ssh_client::Client::connect_install_args(&install_args).await?;
    run_script(
        &client,
        &build_cleanup_script(&spec.apply_policy.server_dir, &spec.workspace_paths),
        "cleanup remote proxy settings",
    )
    .await?;
    let _ = remote_url;
    Ok(())
}

pub(super) fn setup_hash(payload: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(payload.to_string().as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn setup_payload(spec: &ProxySessionSpec, remote_url: &str, route: Option<&Value>) -> Value {
    let env = build_proxy_env(remote_url, &spec.apply_policy.no_proxy);
    let mut values = serde_json::Map::new();
    values.insert("http.proxy".to_string(), json!(remote_url));
    values.insert(
        "http.proxySupport".to_string(),
        json!(&spec.apply_policy.proxy_support),
    );
    if spec.apply_policy.terminal_env {
        values.insert(
            "terminal.integrated.env.linux".to_string(),
            json!(env.clone()),
        );
        values.insert(
            "terminal.integrated.env.osx".to_string(),
            json!(env.clone()),
        );
        values.insert("terminal.integrated.env.windows".to_string(), json!(env));
    }
    json!({
        "target": &spec.target,
        "workspaceId": &spec.workspace_id,
        "workspacePaths": &spec.workspace_paths,
        "proxyUrl": remote_url,
        "bindHost": spec.remote_bind.to_string(),
        "port": spec.remote_port_policy.preferred,
        "connectMode": &spec.connect_mode,
        "routeId": spec.route_id(),
        "jobId": spec.job_id(),
        "routeOwner": route.and_then(|route| route.get("owner")).and_then(Value::as_str),
        "selectedTransport": route.and_then(|route| route.get("selected_transport")).and_then(Value::as_str),
        "fallbackReason": route.and_then(|route| route.get("fallback_reason")).and_then(Value::as_str),
        "localProxySource": "daemon",
        "localProxyUrl": &spec.local_proxy,
        "backend": "ssh_proxy",
        "server_dir": &spec.apply_policy.server_dir,
        "no_proxy": &spec.apply_policy.no_proxy,
        "proxy_support": &spec.apply_policy.proxy_support,
        "values": values,
    })
}

fn build_proxy_env(proxy_url: &str, no_proxy: &str) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("HTTP_PROXY".to_string(), proxy_url.to_string());
    env.insert("HTTPS_PROXY".to_string(), proxy_url.to_string());
    env.insert("ALL_PROXY".to_string(), proxy_url.to_string());
    env.insert("NO_PROXY".to_string(), no_proxy.to_string());
    env.insert("http_proxy".to_string(), proxy_url.to_string());
    env.insert("https_proxy".to_string(), proxy_url.to_string());
    env.insert("all_proxy".to_string(), proxy_url.to_string());
    env.insert("no_proxy".to_string(), no_proxy.to_string());
    env
}

async fn run_script(client: &ssh_client::Client, script: &str, label: &str) -> Result<()> {
    let output = client
        .exec_capture("sh -s".to_string(), Some(script.as_bytes().to_vec()))
        .await
        .with_context(|| format!("{label} failed to start"))?;
    if output.exit_status != 0 {
        let stderr = output.stderr.trim();
        let stdout = output.stdout.trim();
        let detail = match (stderr.is_empty(), stdout.is_empty()) {
            (false, false) => format!("stderr: {stderr}; stdout: {stdout}"),
            (false, true) => stderr.to_string(),
            (true, false) => stdout.to_string(),
            (true, true) => "no output".to_string(),
        };
        return Err(anyhow!(
            "{label} failed with status {}: {}",
            output.exit_status,
            detail
        ));
    }
    Ok(())
}

fn build_remote_settings_script(payload: &Value) -> Result<String> {
    let payload_text = serde_json::to_string(payload)?;
    let payload_js = serde_json::to_string(&payload_text)?;
    let server_dir = payload
        .get("server_dir")
        .and_then(Value::as_str)
        .unwrap_or(".vscode-server");
    Ok(format!(
        r#"
set -eu
node_bin="$(command -v node || true)"
server_dir={server_dir}
if [ -z "$node_bin" ]; then
  for candidate in "$HOME/$server_dir"/bin/*/node "$HOME/$server_dir"/cli/servers/*/server/node; do
    if [ -x "$candidate" ]; then
      node_bin="$candidate"
      break
    fi
  done
fi
if [ -z "$node_bin" ]; then
  echo "remote-proxy: node not found on remote; skipped machine settings" >&2
  exit 0
fi
"$node_bin" <<'NODE'
const fs = require('fs');
const os = require('os');
const path = require('path');
const payload = JSON.parse({payload_js});
const settingsPath = path.join(os.homedir(), payload.server_dir, 'data', 'Machine', 'settings.json');
fs.mkdirSync(path.dirname(settingsPath), {{ recursive: true }});

function stripJsonComments(input) {{
  let output = '';
  let inString = false;
  let quote = '';
  let escaped = false;
  for (let index = 0; index < input.length; index += 1) {{
    const char = input[index];
    const next = input[index + 1];
    if (inString) {{
      output += char;
      if (escaped) {{
        escaped = false;
      }} else if (char === '\\\\') {{
        escaped = true;
      }} else if (char === quote) {{
        inString = false;
      }}
      continue;
    }}
    if (char === '"' || char === "'") {{
      inString = true;
      quote = char;
      output += char;
      continue;
    }}
    if (char === '/' && next === '/') {{
      while (index < input.length && input[index] !== '\n') {{
        index += 1;
      }}
      output += '\n';
      continue;
    }}
    if (char === '/' && next === '*') {{
      index += 2;
      while (index < input.length && !(input[index] === '*' && input[index + 1] === '/')) {{
        index += 1;
      }}
      index += 1;
      continue;
    }}
    output += char;
  }}
  return output;
}}

function parseSettings(raw) {{
  if (!raw.trim()) {{
    return {{}};
  }}
  try {{
    return JSON.parse(raw);
  }} catch {{}}
  try {{
    return JSON.parse(stripJsonComments(raw).replace(/,\s*([}}\]])/g, '$1'));
  }} catch {{
    return {{}};
  }}
}}

const raw = fs.existsSync(settingsPath) ? fs.readFileSync(settingsPath, 'utf8') : '';
if (raw.trim()) {{
  fs.copyFileSync(settingsPath, settingsPath + '.vscode-remote-proxy.bak');
}}
const settings = parseSettings(raw);
for (const [key, value] of Object.entries(payload.values)) {{
  if (key.startsWith('terminal.integrated.env.')) {{
    settings[key] = {{ ...(settings[key] || {{}}), ...value }};
  }} else {{
    settings[key] = value;
  }}
}}
fs.writeFileSync(settingsPath, JSON.stringify(settings, null, 2) + '\n');
console.log('remote-proxy: patched ' + settingsPath);
NODE
"#,
        server_dir = shell_quote(server_dir),
    ))
}

fn build_server_env_setup_script(server_dir: &str, env: &BTreeMap<String, String>) -> String {
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
    format!(
        r#"
set -eu
server_dir={server_dir}
target="$HOME/$server_dir/server-env-setup"
mkdir -p "$(dirname "$target")"
tmp="$target.tmp.$$"
if [ -f "$target" ]; then
  awk '
    /# >>> vscode-remote-proxy >>>/ {{ skip=1; next }}
    /# <<< vscode-remote-proxy <<</ {{ skip=0; next }}
    skip != 1 {{ print }}
  ' "$target" > "$tmp"
else
  : > "$tmp"
fi
printf '%s\n' {block} >> "$tmp"
chmod 600 "$tmp"
mv "$tmp" "$target"
echo "remote-proxy: patched $target"
"#,
        server_dir = shell_quote(server_dir),
        block = shell_quote(&block),
    )
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

fn build_remote_status_file_script(server_dir: &str, payload: &Value) -> Result<String> {
    let encoded = serde_json::to_string(payload)?;
    Ok(format!(
        r#"
set -eu
server_dir={server_dir}
target="$HOME/$server_dir/remote-proxy-status.json"
mkdir -p "$(dirname "$target")"
cat > "$target" <<'JSON'
{encoded}
JSON
chmod 600 "$target"
echo "remote-proxy: wrote $target"
"#,
        server_dir = shell_quote(server_dir),
        encoded = encoded,
    ))
}

fn build_cleanup_script(server_dir: &str, workspace_paths: &[String]) -> String {
    let workspace_lines = workspace_paths
        .iter()
        .map(|workspace_path| format!("cleanup_workspace_git {}", shell_quote(workspace_path)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"
set -eu
server_dir={shell_server_dir}

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

node_bin="$(command -v node || true)"
if [ -z "$node_bin" ]; then
  for candidate in "$HOME/$server_dir"/bin/*/node "$HOME/$server_dir"/cli/servers/*/server/node; do
    if [ -x "$candidate" ]; then
      node_bin="$candidate"
      break
    fi
  done
fi
if [ -z "$node_bin" ]; then
  echo "remote-proxy: node not found on remote; skipped machine settings cleanup" >&2
  exit 0
fi

"$node_bin" <<'NODE'
const fs = require('fs');
const os = require('os');
const path = require('path');
const envKeys = JSON.parse({env_keys});
const settingsPath = path.join(os.homedir(), {js_server_dir}, 'data', 'Machine', 'settings.json');

function stripJsonComments(input) {{
  let output = '';
  let inString = false;
  let quote = '';
  let escaped = false;
  for (let index = 0; index < input.length; index += 1) {{
    const char = input[index];
    const next = input[index + 1];
    if (inString) {{
      output += char;
      if (escaped) {{
        escaped = false;
      }} else if (char === '\\\\') {{
        escaped = true;
      }} else if (char === quote) {{
        inString = false;
      }}
      continue;
    }}
    if (char === '"' || char === "'") {{
      inString = true;
      quote = char;
      output += char;
      continue;
    }}
    if (char === '/' && next === '/') {{
      while (index < input.length && input[index] !== '\n') {{
        index += 1;
      }}
      output += '\n';
      continue;
    }}
    if (char === '/' && next === '*') {{
      index += 2;
      while (index < input.length && !(input[index] === '*' && input[index + 1] === '/')) {{
        index += 1;
      }}
      index += 1;
      continue;
    }}
    output += char;
  }}
  return output;
}}

function parseSettings(raw) {{
  if (!raw.trim()) {{
    return {{}};
  }}
  try {{
    return JSON.parse(raw);
  }} catch {{}}
  try {{
    return JSON.parse(stripJsonComments(raw).replace(/,\s*([}}\]])/g, '$1'));
  }} catch {{
    return {{}};
  }}
}}

function cleanupTerminalEnv(settings, key) {{
  const value = settings[key];
  if (!value || typeof value !== 'object' || Array.isArray(value)) {{
    return;
  }}
  for (const envKey of envKeys) {{
    delete value[envKey];
  }}
  if (Object.keys(value).length === 0) {{
    delete settings[key];
  }} else {{
    settings[key] = value;
  }}
}}

if (!fs.existsSync(settingsPath)) {{
  console.log('remote-proxy: machine settings not found');
  process.exit(0);
}}

const raw = fs.readFileSync(settingsPath, 'utf8');
const settings = parseSettings(raw);
fs.copyFileSync(settingsPath, settingsPath + '.vscode-remote-proxy.cleanup.bak');
delete settings['http.proxy'];
delete settings['http.proxySupport'];
cleanupTerminalEnv(settings, 'terminal.integrated.env.linux');
cleanupTerminalEnv(settings, 'terminal.integrated.env.osx');
cleanupTerminalEnv(settings, 'terminal.integrated.env.windows');
fs.writeFileSync(settingsPath, JSON.stringify(settings, null, 2) + '\n');
console.log('remote-proxy: cleaned ' + settingsPath);
NODE

echo "remote-proxy: cleanup complete"
"#,
        shell_server_dir = shell_quote(server_dir),
        workspace_count = workspace_paths.len(),
        workspace_lines = if workspace_lines.is_empty() {
            "    :".to_string()
        } else {
            workspace_lines
        },
        env_keys = serde_json::to_string(&vec![
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "no_proxy",
        ])
        .map(|value| serde_json::to_string(&value).unwrap())
        .unwrap(),
        js_server_dir = serde_json::to_string(server_dir).unwrap(),
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use crate::cli;

    use super::*;

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
            connect_mode: cli::RouteConnectMode::ReverseLink,
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
    fn remote_setup_does_not_embed_shell_port_verification() {
        let setup_source = include_str!("remote_setup.rs");

        assert!(!setup_source.contains(&["nc", " -z"].concat()));
        assert!(!setup_source.contains(&["/dev", "/tcp"].concat()));
        assert!(!setup_source.contains(&["socket", ".create_connection"].concat()));
    }
}
