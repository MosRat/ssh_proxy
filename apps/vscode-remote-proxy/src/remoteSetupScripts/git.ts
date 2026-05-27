import { shellQuote } from './shell';

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
