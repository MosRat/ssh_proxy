import { ProxyEnvironment } from './env';
import { shellQuote } from './shell';

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
