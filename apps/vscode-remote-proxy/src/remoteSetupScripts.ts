export interface ProxyEnvironment {
  readonly HTTP_PROXY: string;
  readonly HTTPS_PROXY: string;
  readonly ALL_PROXY: string;
  readonly NO_PROXY: string;
  readonly http_proxy: string;
  readonly https_proxy: string;
  readonly all_proxy: string;
  readonly no_proxy: string;
}

export function buildProxyEnv(proxyUrl: string, noProxy: string): ProxyEnvironment {
  return {
    HTTP_PROXY: proxyUrl,
    HTTPS_PROXY: proxyUrl,
    ALL_PROXY: proxyUrl,
    NO_PROXY: noProxy,
    http_proxy: proxyUrl,
    https_proxy: proxyUrl,
    all_proxy: proxyUrl,
    no_proxy: noProxy
  };
}

export function buildVerifyForwardScript(host: string, port: number): string {
  return `
set -eu
host=${shellQuote(host)}
port=${shellQuote(String(port))}
if command -v nc >/dev/null 2>&1; then
  nc -z "$host" "$port"
  exit $?
fi
if command -v python3 >/dev/null 2>&1; then
  python3 - "$host" "$port" <<'PY'
import socket
import sys
host = sys.argv[1]
port = int(sys.argv[2])
sock = socket.create_connection((host, port), timeout=2)
sock.close()
PY
  exit $?
fi
if command -v bash >/dev/null 2>&1; then
  bash -c ":</dev/tcp/$host/$port"
  exit $?
fi
echo "remote-proxy: no nc/python3/bash available to verify forwarded port" >&2
exit 2
`;
}

export function buildRemotePortFreeScript(host: string, port: number): string {
  return `
set -eu
host=${shellQuote(host)}
port=${shellQuote(String(port))}
if command -v python3 >/dev/null 2>&1; then
  python3 - "$host" "$port" <<'PY'
import socket
import sys
host = sys.argv[1]
port = int(sys.argv[2])
sock = socket.socket()
sock.settimeout(1)
try:
    sock.connect((host, port))
except OSError:
    sys.exit(0)
else:
    sys.exit(1)
finally:
    sock.close()
PY
  exit $?
fi
if command -v nc >/dev/null 2>&1; then
  if nc -z "$host" "$port" >/dev/null 2>&1; then
    exit 1
  fi
  exit 0
fi
if command -v bash >/dev/null 2>&1; then
  if bash -c ":</dev/tcp/$host/$port" >/dev/null 2>&1; then
    exit 1
  fi
  exit 0
fi
exit 0
`;
}

export function buildGitConfigScript(options: {
  readonly proxyUrl: string;
  readonly workspacePaths: readonly string[];
  readonly applyGlobal: boolean;
  readonly applyWorkspace: boolean;
  readonly forceOverride: boolean;
}): string {
  const replaceArg = options.forceOverride ? '--replace-all' : '';
  const workspaceLines = options.workspacePaths.map((workspacePath) => `apply_workspace_git ${shellQuote(workspacePath)}`).join('\n');

  return `
set -u
proxy_url=${shellQuote(options.proxyUrl)}
replace_arg=${shellQuote(replaceArg)}

if ! command -v git >/dev/null 2>&1; then
  echo "remote-proxy: git not found on remote; skipped Git proxy config"
  exit 0
fi

apply_git_pair() {
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
}

apply_workspace_git() {
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
}

${options.applyGlobal ? 'apply_git_pair "global" --global || true' : 'echo "remote-proxy: global Git proxy config disabled"'}

if ${options.applyWorkspace ? 'true' : 'false'}; then
  if [ ${options.workspacePaths.length} -eq 0 ]; then
    echo "remote-proxy: no remote workspace folders available for workspace Git proxy config"
  else
${workspaceLines || '    :'}
  fi
else
  echo "remote-proxy: workspace Git proxy config disabled"
fi
`;
}

export function buildRemoteSettingsScript(payload: unknown): string {
  const encoded = Buffer.from(JSON.stringify(payload), 'utf8').toString('base64');
  return `
set -eu
node_bin="$(command -v node || true)"
server_dir="$(node -e "const p=JSON.parse(Buffer.from('${encoded}','base64').toString()).serverDir; console.log(p)" 2>/dev/null || printf '%s' '.vscode-server')"
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
const payload = JSON.parse(Buffer.from('${encoded}', 'base64').toString('utf8'));
const settingsPath = path.join(os.homedir(), payload.serverDir, 'data', 'Machine', 'settings.json');
fs.mkdirSync(path.dirname(settingsPath), { recursive: true });

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

const raw = fs.existsSync(settingsPath) ? fs.readFileSync(settingsPath, 'utf8') : '';
if (raw.trim()) {
  fs.copyFileSync(settingsPath, settingsPath + '.vscode-remote-proxy.bak');
}
const settings = parseSettings(raw);
for (const [key, value] of Object.entries(payload.values)) {
  if (key.startsWith('terminal.integrated.env.')) {
    settings[key] = { ...(settings[key] || {}), ...value };
  } else {
    settings[key] = value;
  }
}
fs.writeFileSync(settingsPath, JSON.stringify(settings, null, 2) + '\\n');
console.log('remote-proxy: patched ' + settingsPath);
NODE
`;
}

export function buildServerEnvSetupScript(serverDir: string, env: ProxyEnvironment): string {
  const lines = Object.entries(env).map(([key, value]) => `export ${key}=${shellQuote(value)}`);
  const block = [
    '# >>> vscode-remote-proxy >>>',
    ...lines,
    '# <<< vscode-remote-proxy <<<'
  ].join('\n');

  return `
set -eu
target="$HOME/${serverDir}/server-env-setup"
mkdir -p "$(dirname "$target")"
tmp="$target.tmp.$$"
if [ -f "$target" ]; then
  awk '
    /# >>> vscode-remote-proxy >>>/ { skip=1; next }
    /# <<< vscode-remote-proxy <<</ { skip=0; next }
    skip != 1 { print }
  ' "$target" > "$tmp"
else
  : > "$tmp"
fi
printf '%s\\n' ${shellQuote(block)} >> "$tmp"
chmod 600 "$tmp"
mv "$tmp" "$target"
echo "remote-proxy: patched $target"
`;
}

export function buildRemoteStatusFileScript(
  serverDir: string,
  payload: {
    readonly proxyUrl: string;
    readonly bindHost: string;
    readonly port: number;
    readonly updatedAt: string;
    readonly localProxySource: string;
    readonly localProxyUrl?: string;
    readonly backend?: string;
    readonly routeId?: string;
    readonly routeOwner?: string;
    readonly selectedTransport?: string;
    readonly connectMode?: string;
    readonly fallbackReason?: string;
  },
): string {
  const encoded = Buffer.from(JSON.stringify(payload, null, 2), 'utf8').toString('base64');
  return `
set -eu
server_dir=${shellQuote(serverDir)}
target="$HOME/$server_dir/remote-proxy-status.json"
mkdir -p "$(dirname "$target")"
if command -v base64 >/dev/null 2>&1; then
  printf '%s' ${shellQuote(encoded)} | base64 -d > "$target"
else
  python3 - "$target" <<'PY'
import base64
import sys
payload = ${JSON.stringify(encoded)}
with open(sys.argv[1], 'wb') as handle:
    handle.write(base64.b64decode(payload))
PY
fi
chmod 600 "$target"
echo "remote-proxy: wrote $target"
`;
}

export function buildReadRemoteStatusFileScript(serverDir: string): string {
  return `
set -eu
server_dir=${shellQuote(serverDir)}
target="$HOME/$server_dir/remote-proxy-status.json"
if [ ! -f "$target" ]; then
  exit 0
fi
cat "$target"
`;
}

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

export function shellQuote(value: string): string {
  return `'${value.replace(/'/g, `'\\''`)}'`;
}
