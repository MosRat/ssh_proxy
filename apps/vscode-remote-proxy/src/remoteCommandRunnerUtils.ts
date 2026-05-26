import { RemoteProxyConfig } from './types';

export interface SshProxyHostExecJsonResult {
  readonly ok?: boolean;
  readonly exit_code?: number | null;
  readonly stdout?: string;
  readonly stderr?: string;
  readonly timed_out?: boolean;
}

export function buildOpenSshRemoteScriptArgs(config: RemoteProxyConfig, sshHost: string): string[] {
  const args = [...config.sshArgs];
  if (config.sshBatchMode) {
    args.push('-o', 'BatchMode=yes');
  }
  args.push('-o', `ConnectTimeout=${Math.max(1, config.sshConnectTimeout)}`, sshHost, 'sh', '-s');
  return args;
}

export function buildSshProxyHostExecHostArgs(config: RemoteProxyConfig): string[] {
  const args: string[] = [];
  for (const value of config.sshArgs) {
    args.push('--ssh-arg', value);
  }
  return args;
}

export function sshProxyHostExecTimeoutSecs(config: RemoteProxyConfig): number {
  return Math.max(1, config.sshConnectTimeout);
}

export function assertSshProxyHostExecSucceeded(result: unknown, label: string): void {
  const record = asHostExecResult(result);
  if (!record) {
    throw new Error(`${label} failed through ssh_proxy host exec: missing JSON result`);
  }

  if (record.timed_out === true) {
    const detail = formatHostExecDetail(record);
    throw new Error(`${label} timed out through ssh_proxy host exec${detail ? `: ${detail}` : ''}`);
  }

  if (record.ok === false || (typeof record.exit_code === 'number' && record.exit_code !== 0)) {
    const detail = formatHostExecDetail(record);
    throw new Error(`${label} failed through ssh_proxy host exec${detail ? `: ${detail}` : ''}`);
  }
}

export function sshProxyHostExecStdout(result: unknown, label: string): string {
  assertSshProxyHostExecSucceeded(result, label);
  const record = asHostExecResult(result);
  return record?.stdout ?? '';
}

function asHostExecResult(value: unknown): SshProxyHostExecJsonResult | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return undefined;
  }
  return value as SshProxyHostExecJsonResult;
}

function formatHostExecDetail(result: SshProxyHostExecJsonResult): string {
  const parts: string[] = [];
  if (typeof result.exit_code === 'number') {
    parts.push(`exit_code=${result.exit_code}`);
  } else if (result.exit_code === null) {
    parts.push('exit_code=null');
  }
  if (result.timed_out === true) {
    parts.push('timed_out=true');
  }
  const stderr = result.stderr?.trim();
  if (stderr) {
    parts.push(stderr);
  }
  const stdout = result.stdout?.trim();
  if (stdout) {
    parts.push(`stdout: ${stdout}`);
  }
  return parts.join(' ');
}
