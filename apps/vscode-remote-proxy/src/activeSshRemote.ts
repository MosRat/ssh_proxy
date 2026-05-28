import * as vscode from 'vscode';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { SshTargetConfig } from './types';
import { DetectedSshHost, parseSshAuthority } from './vscodeStorage';

export async function detectActiveSshRemoteCommand(): Promise<DetectedSshHost | undefined> {
  let result: unknown;
  try {
    result = await vscode.commands.executeCommand('remote-internal.getActiveSshRemote');
  } catch {
    return undefined;
  }

  const parsed = parseCommandResult(result);
  if (!parsed) {
    return undefined;
  }

  return {
    ...parsed,
    source: `remote-internal.getActiveSshRemote -> ${summarizeResult(result)}`,
    confidence: 'high',
    targetKey: buildTargetKey(result, parsed.host)
  };
}

function parseCommandResult(value: unknown): { host: string; authority: string; sshTarget?: SshTargetConfig } | undefined {
  if (typeof value === 'string') {
    return parseString(value);
  }

  if (Array.isArray(value)) {
    for (const item of value) {
      const parsed = parseCommandResult(item);
      if (parsed) {
        return parsed;
      }
    }
    return undefined;
  }

  if (!value || typeof value !== 'object') {
    return undefined;
  }

  const record = value as Record<string, unknown>;
  for (const key of ['remoteAuthority', 'authority', 'host', 'hostName', 'hostname', 'remoteHost', 'remoteName']) {
    const parsed = parseCommandResult(record[key]);
    if (parsed) {
      return {
        ...parsed,
        sshTarget: parseSshTargetConfig(record.config),
      };
    }
  }

  return undefined;
}

function parseSshTargetConfig(value: unknown): SshTargetConfig | undefined {
  const config = value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : undefined;
  const result: SshTargetConfig = {
    hostName: stringify(config?.HostName ?? config?.hostname),
    user: stringify(config?.User ?? config?.user),
    port: parsePort(config?.Port ?? config?.port),
    identityFiles: normalizePathList(config?.IdentityFile ?? config?.identityFile),
    configFile: firstExistingPath([
      stringify(config?.ConfigFile ?? config?.configFile),
      vscode.workspace.getConfiguration('remote.SSH').get<string>('configFile', ''),
      path.join(os.homedir(), '.ssh', 'config'),
    ]),
    knownHostsFile: firstExistingPath([
      stringify(config?.UserKnownHostsFile ?? config?.userKnownHostsFile),
      path.join(os.homedir(), '.ssh', 'known_hosts'),
    ]),
    proxyJump: normalizeStringList(config?.ProxyJump ?? config?.proxyJump)
      .flatMap((entry) => entry.split(',').map((part) => part.trim()).filter(Boolean)),
    acceptNew: true,
  };
  return hasSshTargetConfig(result) ? result : undefined;
}

function parsePort(value: unknown): number | undefined {
  if (typeof value === 'number' && Number.isInteger(value) && value > 0 && value <= 65535) {
    return value;
  }
  if (typeof value === 'string' && value.trim()) {
    const parsed = Number.parseInt(value.trim(), 10);
    if (Number.isInteger(parsed) && parsed > 0 && parsed <= 65535) {
      return parsed;
    }
  }
  return undefined;
}

function normalizeStringList(value: unknown): string[] {
  if (Array.isArray(value)) {
    return value.flatMap(normalizeStringList);
  }
  const text = stringify(value);
  return text ? [text] : [];
}

function normalizePathList(value: unknown): string[] {
  return normalizeStringList(value).map(expandHome);
}

function firstExistingPath(candidates: Array<string | undefined>): string | undefined {
  for (const candidate of candidates) {
    if (!candidate) {
      continue;
    }
    const expanded = expandHome(candidate);
    if (fs.existsSync(expanded)) {
      return expanded;
    }
  }
  return undefined;
}

function expandHome(value: string): string {
  if (value === '~') {
    return os.homedir();
  }
  if (value.startsWith('~/') || value.startsWith(`~${path.sep}`)) {
    return path.join(os.homedir(), value.slice(2));
  }
  return value;
}

function hasSshTargetConfig(value: SshTargetConfig): boolean {
  return Boolean(
    value.hostName
      || value.user
      || value.port
      || value.identityFiles?.length
      || value.configFile
      || value.knownHostsFile
      || value.proxyJump?.length
      || value.acceptNew,
  );
}

function parseString(value: string): { host: string; authority: string } | undefined {
  const trimmed = value.trim();
  if (!trimmed) {
    return undefined;
  }

  const authority = parseSshAuthority(trimmed);
  if (authority) {
    return authority;
  }

  if (/^[\w.-]+(?:@[\w.-]+)?(?::\d+)?$/.test(trimmed)) {
    return {
      host: trimmed,
      authority: `ssh-remote+${encodeURIComponent(trimmed)}`
    };
  }

  return undefined;
}

function summarizeResult(value: unknown): string {
  if (typeof value === 'string') {
    return value;
  }

  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function buildTargetKey(value: unknown, fallbackHost: string): string {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return fallbackHost;
  }

  const record = value as Record<string, unknown>;
  const config = record.config && typeof record.config === 'object' && !Array.isArray(record.config)
    ? record.config as Record<string, unknown>
    : undefined;
  const user = stringify(config?.User ?? config?.user);
  const hostName = stringify(config?.HostName ?? config?.hostname ?? config?.Host ?? record.hostName ?? record.host);
  return `${user ? `${user}@` : ''}${hostName || fallbackHost}`;
}

function stringify(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}
