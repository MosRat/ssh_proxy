export const DEFAULT_SSH_PROXY_EXECUTABLE = 'ssh_proxy';

export interface SshProxyControlConnection {
  readonly endpoint?: string;
  readonly token?: string;
}

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

export function buildSshProxyServiceStatusArgs(): string[] {
  return ['service', '--json', 'status'];
}

export function buildSshProxyServiceInstallArgs(scope: 'auto' | 'user' | 'system' = 'auto'): string[] {
  return ['service', '--scope', scope, 'install'];
}

export function buildSshProxyNodeControlStatusArgs(connection: SshProxyControlConnection = {}): string[] {
  return buildSshProxyNodeControlArgs(connection, ['status']);
}

export function buildSshProxyNodeControlShutdownArgs(connection: SshProxyControlConnection = {}): string[] {
  return buildSshProxyNodeControlArgs(connection, ['shutdown']);
}

export function buildSshProxyStopRouteArgs(routeId: string, connection: SshProxyControlConnection = {}): string[] {
  return buildSshProxyNodeControlArgs(connection, ['stop-route', routeId]);
}

export function buildSshProxyRoutesArgs(connection: SshProxyControlConnection = {}): string[] {
  return buildSshProxyNodeControlArgs(connection, ['routes']);
}

export function buildSshProxyNodeDaemonArgs(options: {
  readonly control: string;
  readonly transport: string;
  readonly token: string;
  readonly name?: string;
}): string[] {
  const args = [
    'node',
    'daemon',
    '--control',
    options.control,
    '--transport',
    options.transport,
    '--token',
    options.token,
    '--no-route-autostart',
  ];
  if (options.name) {
    args.push('--name', options.name);
  }
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

function buildSshProxyNodeControlArgs(connection: SshProxyControlConnection, command: readonly string[]): string[] {
  const args = ['node', 'control'];
  if (connection.endpoint) {
    args.push('--endpoint', connection.endpoint);
  }
  if (connection.token) {
    args.push('--token', connection.token);
  }
  args.push('--json', ...command);
  return args;
}
