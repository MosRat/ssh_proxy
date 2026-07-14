import type { SshHostEntry } from './sshConfig';
import type { LocalProxy } from './types';

export interface LocalProxyQuickPickItem {
  readonly label: string;
  readonly description?: string;
  readonly candidate?: LocalProxy;
}

export interface SshHostQuickPickItem {
  readonly label: string;
  readonly description?: string;
  readonly picked?: boolean;
  readonly entry?: SshHostEntry;
  readonly manual?: boolean;
}

export function buildLocalProxyPickItems(candidates: readonly LocalProxy[]): LocalProxyQuickPickItem[] {
  return [
    ...candidates.map((candidate) => ({
      label: candidate.url,
      description: candidate.source,
      candidate,
    })),
    {
      label: 'Enter proxy URL...',
      description: 'http://127.0.0.1:<port> or socks5://127.0.0.1:<port>',
    },
  ];
}

export function buildSshHostPickItems(
  entries: readonly SshHostEntry[],
  current: string | undefined,
): SshHostQuickPickItem[] {
  return [
    ...entries.map((entry) => ({
      label: entry.alias,
      description: entry.source,
      picked: entry.alias === current,
      entry,
    })),
    {
      label: 'Enter SSH host...',
      description: 'Use the same host alias you use with ssh or Remote SSH',
      manual: true,
    },
  ];
}

export function sshHostPickPlaceholder(current: string | undefined): string {
  return current ? `Current: ${current}` : 'Select a Host from ~/.ssh/config or enter one manually';
}
