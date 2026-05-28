export const DEFAULT_SSH_PROXY_EXECUTABLE = 'ssh_proxy';

const SECRET_VALUE_FLAGS = new Set([
  '--token',
  '--remote-token',
  '--auth-token',
  '--password',
  '--passphrase',
]);

export function normalizeSshProxyExecutable(configured: string | undefined | null): string {
  const trimmed = (configured ?? '').trim();
  if (!trimmed) {
    return DEFAULT_SSH_PROXY_EXECUTABLE;
  }
  if (
    (trimmed.startsWith('"') && trimmed.endsWith('"')) ||
    (trimmed.startsWith("'") && trimmed.endsWith("'"))
  ) {
    return trimmed.slice(1, -1);
  }
  return trimmed;
}

export function buildSshProxyVscodeUpArgs(options: {
  readonly target: string;
  readonly workspace: string;
  readonly localProxy: string;
  readonly remoteBind: string;
  readonly remotePort: number;
  readonly connectMode: 'auto' | 'reverse-link' | 'direct';
  readonly sshTarget?: {
    readonly hostName?: string;
    readonly user?: string;
    readonly port?: number;
    readonly identityFiles?: readonly string[];
    readonly configFile?: string;
    readonly knownHostsFile?: string;
    readonly proxyJump?: readonly string[];
    readonly acceptNew?: boolean;
  };
  readonly workspacePaths?: readonly string[];
  readonly serverDir?: string;
  readonly noProxy?: string;
  readonly proxySupport?: string;
  readonly applyRemoteMachineSettings?: boolean;
  readonly applyTerminalEnv?: boolean;
  readonly applyServerEnvSetup?: boolean;
  readonly applyGitConfig?: boolean;
  readonly applyGitGlobalConfig?: boolean;
  readonly applyGitWorkspaceConfig?: boolean;
  readonly applyGitForceOverride?: boolean;
  readonly applyRemoteStatusFile?: boolean;
  readonly verifyRemotePort?: boolean;
}): string[] {
  const args = [
    'vscode',
    'up',
    '--target',
    options.target,
    '--workspace',
    options.workspace,
    '--local-proxy',
    options.localProxy,
    '--remote-bind',
    options.remoteBind,
    '--remote-port',
    String(options.remotePort),
    '--connect-mode',
    options.connectMode,
  ];
  if (options.sshTarget?.hostName) {
    args.push('--ssh-host-name', options.sshTarget.hostName);
  }
  if (options.sshTarget?.user) {
    args.push('--ssh-user', options.sshTarget.user);
  }
  if (options.sshTarget?.port) {
    args.push('--ssh-port', String(options.sshTarget.port));
  }
  for (const identity of options.sshTarget?.identityFiles ?? []) {
    args.push('--ssh-identity', identity);
  }
  if (options.sshTarget?.configFile) {
    args.push('--ssh-config', options.sshTarget.configFile);
  }
  if (options.sshTarget?.knownHostsFile) {
    args.push('--ssh-known-hosts', options.sshTarget.knownHostsFile);
  }
  for (const jump of options.sshTarget?.proxyJump ?? []) {
    args.push('--ssh-jump', jump);
  }
  if (options.sshTarget?.acceptNew) {
    args.push('--ssh-accept-new');
  }
  for (const workspacePath of options.workspacePaths ?? []) {
    args.push('--workspace-path', workspacePath);
  }
  if (options.serverDir) {
    args.push('--server-dir', options.serverDir);
  }
  if (options.noProxy) {
    args.push('--no-proxy', options.noProxy);
  }
  if (options.proxySupport) {
    args.push('--proxy-support', options.proxySupport);
  }
  if (options.applyRemoteMachineSettings === false) {
    args.push('--no-remote-machine-settings');
  }
  if (options.applyTerminalEnv === false) {
    args.push('--no-terminal-env');
  }
  if (options.applyServerEnvSetup === false) {
    args.push('--no-server-env');
  }
  if (options.applyGitConfig === false) {
    args.push('--no-git');
  }
  if (options.applyGitGlobalConfig === false) {
    args.push('--no-git-global');
  }
  if (options.applyGitWorkspaceConfig === false) {
    args.push('--no-git-workspace');
  }
  if (options.applyGitForceOverride === false) {
    args.push('--no-git-force-override');
  }
  if (options.applyRemoteStatusFile === false) {
    args.push('--no-remote-status-file');
  }
  if (options.verifyRemotePort === false) {
    args.push('--no-verify-remote-port');
  }
  args.push('--json');
  return args;
}

export function buildSshProxyVscodeStatusArgs(options: {
  readonly workspace?: string;
  readonly target?: string;
}): string[] {
  const args = ['vscode', 'status'];
  if (options.workspace) {
    args.push('--workspace', options.workspace);
  }
  if (options.target) {
    args.push('--target', options.target);
  }
  args.push('--json');
  return args;
}

export function buildSshProxyVscodeApplySettingsArgs(options: {
  readonly target: string;
  readonly workspace: string;
  readonly proxyUrl: string;
}): string[] {
  return [
    'vscode',
    'apply-settings',
    '--target',
    options.target,
    '--workspace',
    options.workspace,
    '--proxy-url',
    options.proxyUrl,
    '--json',
  ];
}

export function buildSshProxyDaemonInstallArgs(options: {
  readonly scope: 'system' | 'user';
  readonly elevate?: boolean;
  readonly json?: boolean;
}): string[] {
  const args = ['daemon', 'install', '--scope', options.scope];
  if (options.elevate) {
    args.push('--elevate');
  }
  if (options.json) {
    args.push('--json');
  }
  return args;
}

export function buildSshProxyDoctorArgs(options: {
  readonly report?: boolean;
  readonly json?: boolean;
} = {}): string[] {
  const args = ['doctor'];
  if (options.report) {
    args.push('--report');
  }
  if (options.json) {
    args.push('--json');
  }
  return args;
}

export function isSshProxyDaemonInstallCancelledMessage(message: string): boolean {
  const lower = message.toLowerCase();
  return lower.includes('0xc000013a')
    || lower.includes('cancelled_by_user')
    || lower.includes('exit code 1223')
    || lower.includes('code 1223')
    || lower.includes('operation was canceled')
    || lower.includes('operation was cancelled')
    || lower.includes('the operation was canceled by the user')
    || lower.includes('用户取消');
}

export function buildSshProxyDownArgs(options: {
  readonly routeId?: string;
  readonly workspace?: string;
  readonly target?: string;
}): string[] {
  const args = ['down'];
  if (options.routeId) {
    args.push('--route-id', options.routeId);
  }
  if (options.workspace) {
    args.push('--workspace', options.workspace);
  }
  if (options.target) {
    args.push('--target', options.target);
  }
  args.push('--json');
  return args;
}

export function redactSshProxyArgs(args: readonly string[]): string[] {
  const redacted: string[] = [];
  let redactNext = false;
  for (const arg of args) {
    if (redactNext) {
      redacted.push('<redacted>');
      redactNext = false;
      continue;
    }

    const inlineFlag = arg.match(/^(--[^=]+)=(.*)$/);
    if (inlineFlag && SECRET_VALUE_FLAGS.has(inlineFlag[1])) {
      redacted.push(`${inlineFlag[1]}=<redacted>`);
      continue;
    }

    redacted.push(redactSshProxyText(arg));
    if (SECRET_VALUE_FLAGS.has(arg)) {
      redactNext = true;
    }
  }
  return redacted;
}

export function formatSshProxyCommand(executable: string, args: readonly string[]): string {
  return [normalizeSshProxyExecutable(executable), ...redactSshProxyArgs(args)]
    .map(quoteCommandPart)
    .join(' ');
}

export function parseSshProxyJson(text: string, label = 'ssh_proxy'): unknown {
  const trimmed = text.trim();
  if (!trimmed) {
    return null;
  }
  try {
    return JSON.parse(trimmed);
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    const preview = summarizeSshProxyOutput(trimmed);
    throw new Error(`${label} did not return valid JSON: ${detail}${preview ? `\n${preview}` : ''}`);
  }
}

export function summarizeSshProxyOutput(text: string, limit = 4096): string {
  const redacted = redactSshProxyText(text.trim());
  if (redacted.length <= limit) {
    return redacted;
  }
  return `${redacted.slice(0, limit)}...<truncated>`;
}

export function redactSshProxyText(text: string): string {
  return text.replace(
    /\b([A-Za-z][A-Za-z0-9+.-]*:\/\/)([^@\s/]+)@/g,
    (_match, scheme: string) => `${scheme}<redacted>@`,
  );
}

function quoteCommandPart(part: string): string {
  if (part.length > 0 && !/\s/.test(part)) {
    return part;
  }
  return `"${part.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;
}
