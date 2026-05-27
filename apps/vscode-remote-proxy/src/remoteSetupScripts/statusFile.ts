import { shellQuote } from './shell';

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
