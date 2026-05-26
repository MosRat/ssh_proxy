import * as fs from 'fs/promises';
import * as os from 'os';
import * as path from 'path';
import * as vscode from 'vscode';

export interface SshHostEntry {
  readonly alias: string;
  readonly source: string;
}

export async function readSshHostEntries(): Promise<SshHostEntry[]> {
  const files = getCandidateConfigFiles();
  const entries: SshHostEntry[] = [];
  const seen = new Set<string>();

  for (const file of files) {
    const normalizedFile = expandHome(file);
    let text: string;
    try {
      text = await fs.readFile(normalizedFile, 'utf8');
    } catch {
      continue;
    }

    for (const alias of parseHostAliases(text)) {
      const key = alias.toLowerCase();
      if (!seen.has(key)) {
        seen.add(key);
        entries.push({ alias, source: normalizedFile });
      }
    }
  }

  return entries.sort((left, right) => left.alias.localeCompare(right.alias));
}

function getCandidateConfigFiles(): string[] {
  const files: string[] = [];
  const remoteSshConfig = vscode.workspace.getConfiguration('remote.SSH').get<string>('configFile', '');
  if (remoteSshConfig.trim()) {
    files.push(remoteSshConfig.trim());
  }
  files.push(path.join(os.homedir(), '.ssh', 'config'));
  return [...new Set(files)];
}

function parseHostAliases(text: string): string[] {
  const aliases: string[] = [];
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith('#')) {
      continue;
    }

    const match = /^Host\s+(.+)$/i.exec(line);
    if (!match) {
      continue;
    }

    for (const token of splitSshConfigWords(match[1])) {
      if (token && !token.includes('*') && !token.includes('?') && token !== '!') {
        aliases.push(token.startsWith('!') ? token.slice(1) : token);
      }
    }
  }
  return aliases;
}

function splitSshConfigWords(value: string): string[] {
  const words: string[] = [];
  let current = '';
  let quote: '"' | "'" | undefined;

  for (let index = 0; index < value.length; index += 1) {
    const char = value[index];
    if (quote) {
      if (char === quote) {
        quote = undefined;
      } else {
        current += char;
      }
      continue;
    }

    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }

    if (/\s/.test(char)) {
      if (current) {
        words.push(current);
        current = '';
      }
      continue;
    }

    current += char;
  }

  if (current) {
    words.push(current);
  }

  return words;
}

function expandHome(file: string): string {
  if (file === '~') {
    return os.homedir();
  }

  if (file.startsWith(`~${path.sep}`) || file.startsWith('~/')) {
    return path.join(os.homedir(), file.slice(2));
  }

  return file;
}
