export interface ParsedSshAuthority {
  readonly host: string;
  readonly authority: string;
}

export function parseSshAuthority(authority: string): ParsedSshAuthority | undefined {
  const normalized = authority.replace(/^vscode-remote:\/\//, '');
  const prefix = 'ssh-remote+';
  if (!normalized.startsWith(prefix)) {
    return undefined;
  }

  const raw = normalized.slice(prefix.length).split(/[/?#]/, 1)[0];
  if (!raw) {
    return undefined;
  }

  const decoded = decodeUriComponent(raw);
  const host = decodeHexRemoteAuthority(decoded);
  if (host === null) {
    return undefined;
  }

  return {
    host: host ?? decoded,
    authority: `ssh-remote+${raw}`,
  };
}

export function parseSshHostAuthority(authority: string): string | undefined {
  const normalized = authority.startsWith('ssh-remote+') ? authority : `ssh-remote+${authority}`;
  return parseSshAuthority(normalized)?.host;
}

function decodeUriComponent(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

// Recent Remote-SSH versions encode {"hostName":"<ssh config alias>"} as hex.
// null means the value is a JSON envelope but does not contain a usable host.
function decodeHexRemoteAuthority(value: string): string | null | undefined {
  if (value.length % 2 !== 0 || !/^[0-9a-f]+$/i.test(value)) {
    return undefined;
  }

  let decoded: string;
  try {
    decoded = Buffer.from(value, 'hex').toString('utf8');
  } catch {
    return undefined;
  }

  if (!decoded.trimStart().startsWith('{')) {
    return undefined;
  }

  try {
    const parsed: unknown = JSON.parse(decoded);
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
      return null;
    }
    const hostName = (parsed as Record<string, unknown>).hostName;
    return typeof hostName === 'string' && hostName.trim() ? hostName.trim() : null;
  } catch {
    return null;
  }
}
