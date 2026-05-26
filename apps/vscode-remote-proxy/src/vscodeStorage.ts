import * as fs from 'fs/promises';
import * as path from 'path';
import * as vscode from 'vscode';

export interface DetectedSshHost {
  readonly host: string;
  readonly authority: string;
  readonly source: string;
  readonly confidence: 'high' | 'low';
  readonly targetKey?: string;
}

export async function detectSshHostFromVsCodeStorage(
  context: vscode.ExtensionContext,
  options: { includeGlobalStorage: boolean } = { includeGlobalStorage: true }
): Promise<DetectedSshHost | undefined> {
  const workspaceHost = await detectSshHostFromWorkspaceStorage(context);
  if (workspaceHost) {
    return workspaceHost;
  }

  if (!options.includeGlobalStorage) {
    return undefined;
  }

  const globalStorageDir = path.dirname(context.globalStorageUri.fsPath);
  const storagePath = path.join(globalStorageDir, 'storage.json');
  return detectSshHostFromStorageFile(storagePath);
}

async function detectSshHostFromWorkspaceStorage(context: vscode.ExtensionContext): Promise<DetectedSshHost | undefined> {
  const storagePath = await findWorkspaceJson(context.storageUri?.fsPath);
  if (!storagePath) {
    return undefined;
  }

  let storage: unknown;
  try {
    storage = JSON.parse(await fs.readFile(storagePath, 'utf8'));
  } catch {
    return undefined;
  }

  for (const candidate of prioritize(findAuthorityStrings(storage))) {
    const parsed = parseSshAuthority(candidate.value);
    if (!parsed) {
      continue;
    }
    return {
      ...parsed,
      source: `${storagePath}: ${candidate.source}`,
      confidence: 'high'
    };
  }

  return undefined;
}

export async function detectSshHostFromStorageFile(storagePath: string): Promise<DetectedSshHost | undefined> {
  let storage: unknown;
  try {
    storage = JSON.parse(await fs.readFile(storagePath, 'utf8'));
  } catch {
    return undefined;
  }

  const seen = new Set<string>();
  for (const candidate of prioritize(findAuthorityStrings(storage))) {
    const parsed = parseSshAuthority(candidate.value);
    if (!parsed || seen.has(parsed.host)) {
      continue;
    }
    seen.add(parsed.host);
    return {
      ...parsed,
      source: `${storagePath}: ${candidate.source}`,
      confidence: score(candidate.source) >= 700 ? 'high' : 'low'
    };
  }

  return undefined;
}

async function findWorkspaceJson(start: string | undefined): Promise<string | undefined> {
  if (!start) {
    return undefined;
  }

  let current = start;
  for (let depth = 0; depth < 6; depth += 1) {
    const candidate = path.join(current, 'workspace.json');
    try {
      await fs.access(candidate);
      return candidate;
    } catch {
      const parent = path.dirname(current);
      if (parent === current) {
        break;
      }
      current = parent;
    }
  }

  return undefined;
}

export function parseSshAuthority(authority: string): { host: string; authority: string } | undefined {
  const normalized = authority.replace(/^vscode-remote:\/\//, '');
  const prefix = 'ssh-remote+';
  if (!normalized.startsWith(prefix)) {
    return undefined;
  }

  const raw = normalized.slice(prefix.length).split(/[/?#]/, 1)[0];
  if (!raw) {
    return undefined;
  }

  try {
    return { host: decodeURIComponent(raw), authority: `ssh-remote+${raw}` };
  } catch {
    return { host: raw, authority: `ssh-remote+${raw}` };
  }
}

function findAuthorityStrings(value: unknown): Array<{ value: string; source: string }> {
  const results: Array<{ value: string; source: string }> = [];
  walk(value, '$', results);
  return results;
}

function walk(value: unknown, source: string, results: Array<{ value: string; source: string }>): void {
  if (typeof value === 'string') {
    const direct = parseSshAuthority(value);
    if (direct) {
      results.push({ value, source });
      return;
    }

    const fromUri = extractAuthorityFromRemoteUri(value);
    if (fromUri) {
      results.push({ value: fromUri, source });
    }
    return;
  }

  if (Array.isArray(value)) {
    value.forEach((item, index) => walk(item, `${source}[${index}]`, results));
    return;
  }

  if (!value || typeof value !== 'object') {
    return;
  }

  for (const [key, child] of Object.entries(value)) {
    walk(child, `${source}.${key}`, results);
  }
}

function prioritize(items: Array<{ value: string; source: string }>): Array<{ value: string; source: string }> {
  return [...items].sort((left, right) => score(right.source) - score(left.source));
}

function score(source: string): number {
  if (source === '$.windowsState.lastPluginDevelopmentHostWindow.remoteAuthority') {
    return 1100;
  }
  if (source.startsWith('$.windowsState.lastPluginDevelopmentHostWindow.')) {
    return 1050;
  }
  if (source === '$.windowsState.lastActiveWindow.remoteAuthority') {
    return 1000;
  }
  if (source.startsWith('$.windowsState.lastActiveWindow.') && source.endsWith('.remoteAuthority')) {
    return 950;
  }
  if (source.startsWith('$.windowsState.lastActiveWindow.')) {
    return 900;
  }
  if (source.startsWith('$.windowsState.openedWindows') && source.endsWith('.remoteAuthority')) {
    return 800;
  }
  if (source.startsWith('$.windowsState.openedWindows')) {
    return 700;
  }
  if (source.includes('backupWorkspaces') && source.endsWith('.remoteAuthority')) {
    return 100;
  }
  if (source.includes('backupWorkspaces') && source.endsWith('.folderUri')) {
    return 90;
  }
  if (source.includes('windowsState')) {
    return 80;
  }
  if (source.includes('profileAssociations')) {
    return 10;
  }
  return 50;
}

function extractAuthorityFromRemoteUri(uri: string): string | undefined {
  if (!uri.startsWith('vscode-remote://')) {
    return undefined;
  }

  try {
    const parsed = new URL(uri);
    return decodeURIComponent(parsed.hostname);
  } catch {
    const match = /^vscode-remote:\/\/([^/]+)/.exec(uri);
    return match ? decodeURIComponent(match[1]) : undefined;
  }
}
