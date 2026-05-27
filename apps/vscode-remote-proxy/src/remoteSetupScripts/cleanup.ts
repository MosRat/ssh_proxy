import { shellQuote } from './shell';

export function buildCleanupScript(serverDir: string, workspacePaths: readonly string[]): string {
  const proxyEnvKeys = [
    'HTTP_PROXY',
    'HTTPS_PROXY',
    'ALL_PROXY',
    'NO_PROXY',
    'http_proxy',
    'https_proxy',
    'all_proxy',
    'no_proxy'
  ];
  const encodedEnvKeys = Buffer.from(JSON.stringify(proxyEnvKeys), 'utf8').toString('base64');
  const workspaceLines = workspacePaths.map((workspacePath) => `cleanup_workspace_git ${shellQuote(workspacePath)}`).join('\n');

  return `
set -eu
server_dir=${shellQuote(serverDir)}

if command -v git >/dev/null 2>&1; then
  git config --global --unset-all http.proxy >/dev/null 2>&1 || true
  git config --global --unset-all https.proxy >/dev/null 2>&1 || true
  echo "remote-proxy: cleaned global Git proxy config"

  cleanup_workspace_git() {
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
  }

  if [ ${workspacePaths.length} -eq 0 ]; then
    echo "remote-proxy: no remote workspace folders available for workspace Git cleanup"
  else
${workspaceLines || '    :'}
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
    /# >>> vscode-remote-proxy >>>/ { skip=1; next }
    /# <<< vscode-remote-proxy <<</ { skip=0; next }
    skip != 1 { print }
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
const envKeys = JSON.parse(Buffer.from('${encodedEnvKeys}', 'base64').toString('utf8'));
const settingsPath = path.join(os.homedir(), ${JSON.stringify(serverDir)}, 'data', 'Machine', 'settings.json');

function stripJsonComments(input) {
  let output = '';
  let inString = false;
  let quote = '';
  let escaped = false;
  for (let index = 0; index < input.length; index += 1) {
    const char = input[index];
    const next = input[index + 1];
    if (inString) {
      output += char;
      if (escaped) {
        escaped = false;
      } else if (char === '\\\\') {
        escaped = true;
      } else if (char === quote) {
        inString = false;
      }
      continue;
    }
    if (char === '"' || char === "'") {
      inString = true;
      quote = char;
      output += char;
      continue;
    }
    if (char === '/' && next === '/') {
      while (index < input.length && input[index] !== '\\n') {
        index += 1;
      }
      output += '\\n';
      continue;
    }
    if (char === '/' && next === '*') {
      index += 2;
      while (index < input.length && !(input[index] === '*' && input[index + 1] === '/')) {
        index += 1;
      }
      index += 1;
      continue;
    }
    output += char;
  }
  return output;
}

function parseSettings(raw) {
  if (!raw.trim()) {
    return {};
  }
  try {
    return JSON.parse(raw);
  } catch {}
  try {
    return JSON.parse(stripJsonComments(raw).replace(/,\\s*([}\\]])/g, '$1'));
  } catch {
    return {};
  }
}

function cleanupTerminalEnv(settings, key) {
  const value = settings[key];
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return;
  }
  for (const envKey of envKeys) {
    delete value[envKey];
  }
  if (Object.keys(value).length === 0) {
    delete settings[key];
  } else {
    settings[key] = value;
  }
}

if (!fs.existsSync(settingsPath)) {
  console.log('remote-proxy: machine settings not found');
  process.exit(0);
}

const raw = fs.readFileSync(settingsPath, 'utf8');
const settings = parseSettings(raw);
fs.copyFileSync(settingsPath, settingsPath + '.vscode-remote-proxy.cleanup.bak');
delete settings['http.proxy'];
delete settings['http.proxySupport'];
cleanupTerminalEnv(settings, 'terminal.integrated.env.linux');
cleanupTerminalEnv(settings, 'terminal.integrated.env.osx');
cleanupTerminalEnv(settings, 'terminal.integrated.env.windows');
fs.writeFileSync(settingsPath, JSON.stringify(settings, null, 2) + '\\n');
console.log('remote-proxy: cleaned ' + settingsPath);
NODE

echo "remote-proxy: cleanup complete"
`;
}
