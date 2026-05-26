import * as vscode from 'vscode';
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

function parseCommandResult(value: unknown): { host: string; authority: string } | undefined {
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
      return parsed;
    }
  }

  return undefined;
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
