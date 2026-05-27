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
