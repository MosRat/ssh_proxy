import { AppliedProxy, ForwardingBackendKind } from './types';

export type ProxyLeaseVersion = 1 | 2;
export type ProxyLeaseBackend = Exclude<ForwardingBackendKind, 'auto'>;

export interface ProxyLeaseState {
  readonly version: ProxyLeaseVersion;
  readonly ownerId: string;
  readonly pid: number;
  readonly targetKey: string;
  readonly sshHost: string;
  readonly updatedAt: number;
  readonly startedAt: number;
  readonly backend: ProxyLeaseBackend;
  readonly routeId?: string;
  readonly routeOwner?: string;
  readonly selectedTransport?: string;
  readonly remoteUrl: string;
  readonly proxy: AppliedProxy;
}

export function buildProxyLeaseState(input: {
  readonly ownerId: string;
  readonly pid: number;
  readonly targetKey: string;
  readonly sshHost: string;
  readonly proxy: AppliedProxy;
  readonly previous?: ProxyLeaseState;
  readonly now: number;
}): ProxyLeaseState {
  const backend = normalizeLeaseBackend(input.proxy.backend);
  const proxy = normalizeProxyForBackend(input.proxy, backend);
  return {
    version: 2,
    ownerId: input.ownerId,
    pid: input.pid,
    targetKey: input.targetKey,
    sshHost: input.sshHost,
    backend,
    routeId: proxy.routeId,
    routeOwner: proxy.routeOwner ?? backend,
    selectedTransport: proxy.selectedTransport,
    remoteUrl: proxy.remoteUrl,
    proxy,
    startedAt: input.previous?.ownerId === input.ownerId ? input.previous.startedAt : input.now,
    updatedAt: input.now,
  };
}

export function normalizeProxyLeaseState(value: unknown): ProxyLeaseState | undefined {
  const record = asRecord(value);
  if (!record) {
    return undefined;
  }
  const version = record.version === 1 || record.version === 2 ? record.version : undefined;
  const proxy = asAppliedProxy(record.proxy);
  const ownerId = asString(record.ownerId);
  const pid = asNumber(record.pid);
  const targetKey = asString(record.targetKey);
  const sshHost = asString(record.sshHost);
  const updatedAt = asNumber(record.updatedAt);
  const startedAt = asNumber(record.startedAt);
  if (!version || !proxy || !ownerId || pid === undefined || !targetKey || !sshHost || updatedAt === undefined || startedAt === undefined) {
    return undefined;
  }

  if (version === 1) {
    return {
      version,
      ownerId,
      pid,
      targetKey,
      sshHost,
      updatedAt,
      startedAt,
      backend: 'openssh',
      routeId: asString(record.routeId ?? proxy.routeId),
      routeOwner: asString(record.routeOwner ?? proxy.routeOwner) ?? 'openssh',
      selectedTransport: asString(record.selectedTransport ?? proxy.selectedTransport) ?? 'openssh-reverse',
      remoteUrl: asString(record.remoteUrl ?? proxy.remoteUrl) ?? proxy.remoteUrl,
      proxy: normalizeProxyForBackend(proxy, 'openssh'),
    };
  }

  const backend = normalizeLeaseBackend(asString(record.backend) ?? proxy.backend);
  const normalizedProxy = normalizeProxyForBackend(proxy, backend);
  return {
    version,
    ownerId,
    pid,
    targetKey,
    sshHost,
    updatedAt,
    startedAt,
    backend,
    routeId: asString(record.routeId ?? normalizedProxy.routeId),
    routeOwner: asString(record.routeOwner ?? normalizedProxy.routeOwner) ?? backend,
    selectedTransport: asString(record.selectedTransport ?? normalizedProxy.selectedTransport),
    remoteUrl: asString(record.remoteUrl ?? normalizedProxy.remoteUrl) ?? normalizedProxy.remoteUrl,
    proxy: normalizedProxy,
  };
}

function normalizeLeaseBackend(value: unknown): ProxyLeaseBackend {
  return value === 'ssh_proxy' ? 'ssh_proxy' : 'openssh';
}

function normalizeProxyForBackend(proxy: AppliedProxy, backend: ProxyLeaseBackend): AppliedProxy {
  const selectedTransport = proxy.selectedTransport ?? (backend === 'openssh' ? 'openssh-reverse' : undefined);
  return {
    ...proxy,
    backend,
    routeOwner: proxy.routeOwner ?? backend,
    selectedTransport,
  };
}

function asAppliedProxy(value: unknown): AppliedProxy | undefined {
  const record = asRecord(value);
  const local = asRecord(record?.local);
  const remoteUrl = asString(record?.remoteUrl);
  const remotePort = asNumber(record?.remotePort);
  const remoteBindHost = asString(record?.remoteBindHost);
  if (!record || !local || !remoteUrl || remotePort === undefined || !remoteBindHost) {
    return undefined;
  }
  const localUrl = asString(local.url);
  const localScheme = asString(local.scheme);
  const localHost = asString(local.host);
  const localPort = asNumber(local.port);
  const localSource = asString(local.source);
  if (!localUrl || !localScheme || !localHost || localPort === undefined || !localSource) {
    return undefined;
  }
  return {
    local: {
      url: localUrl,
      scheme: localScheme as AppliedProxy['local']['scheme'],
      host: localHost,
      port: localPort,
      source: localSource,
    },
    remoteUrl,
    remotePort,
    remoteBindHost,
    routeId: asString(record.routeId),
    routeOwner: asString(record.routeOwner),
    selectedTransport: asString(record.selectedTransport),
    connectMode: asString(record.connectMode),
    fallbackReason: asString(record.fallbackReason),
    backend: normalizeLeaseBackend(asString(record.backend)),
    cleanupCommand: asString(record.cleanupCommand),
  };
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
}

function asString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}

function asNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}
