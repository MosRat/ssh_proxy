export interface RemoteProxyStatusFile {
  readonly proxyUrl?: string;
  readonly bindHost?: string;
  readonly port?: number;
  readonly updatedAt?: string;
  readonly localProxySource?: string;
  readonly localProxyUrl?: string;
  readonly backend?: string;
  readonly routeId?: string;
  readonly routeOwner?: string;
  readonly selectedTransport?: string;
  readonly connectMode?: string;
  readonly fallbackReason?: string;
}

export function parseRemoteProxyStatusFile(raw: string): RemoteProxyStatusFile | undefined {
  const trimmed = raw.trim();
  if (!trimmed) {
    return undefined;
  }
  try {
    const parsed = JSON.parse(trimmed);
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
      return undefined;
    }
    const record = parsed as Record<string, unknown>;
    return {
      proxyUrl: asString(record.proxyUrl),
      bindHost: asString(record.bindHost),
      port: asNumber(record.port),
      updatedAt: asString(record.updatedAt),
      localProxySource: asString(record.localProxySource),
      localProxyUrl: asString(record.localProxyUrl),
      backend: asString(record.backend),
      routeId: asString(record.routeId),
      routeOwner: asString(record.routeOwner),
      selectedTransport: asString(record.selectedTransport),
      connectMode: asString(record.connectMode),
      fallbackReason: asString(record.fallbackReason),
    };
  } catch {
    return undefined;
  }
}

function asString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}

function asNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isInteger(value) ? value : undefined;
}
