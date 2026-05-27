import { AppliedProxy, SshProxyConnectMode } from './types';

export interface SshProxyRouteState {
  readonly routeId: string;
  readonly owner: string | undefined;
  readonly selectedTransport: string | undefined;
  readonly connectMode: string | undefined;
  readonly fallbackReason: string | undefined;
  readonly remoteUrl: string | undefined;
  readonly cleanupCommand: string | undefined;
  readonly health: unknown;
  readonly liveRoute: Record<string, unknown> | undefined;
}

export interface SshProxyKernelStatusSnapshot {
  readonly serviceStatus: unknown;
  readonly routeExplain: unknown;
  readonly routeStart: unknown;
  readonly routeStop: unknown;
  readonly routeState: SshProxyRouteState | undefined;
  readonly lastRefreshAt: number | undefined;
}

export function emptySshProxyKernelStatusSnapshot(): SshProxyKernelStatusSnapshot {
  return {
    serviceStatus: undefined,
    routeExplain: undefined,
    routeStart: undefined,
    routeStop: undefined,
    routeState: undefined,
    lastRefreshAt: undefined,
  };
}

export function isSshProxyOk(value: unknown): boolean | undefined {
  const record = asRecord(value);
  return typeof record?.ok === 'boolean' ? record.ok : undefined;
}

export function createSshProxyRouteState(
  started: unknown,
  proxy: AppliedProxy,
  defaultConnectMode: SshProxyConnectMode,
): SshProxyRouteState {
  const record = asRecord(started);
  const plan = asRecord(record?.plan);
  const route = asRecord(record?.route);
  const routeId = asString(record?.route_id ?? record?.id ?? plan?.route_id)
    ?? asString(route?.route_id ?? route?.id)
    ?? proxy.routeId
    ?? routeIdFrom(proxy.remoteUrl);
  const selectedTransport = asString(
    record?.selected_transport
    ?? plan?.selected_transport
    ?? plan?.selectedTransport
    ?? proxy.selectedTransport,
  );
  const fallbackReason = asString(
    record?.fallback_reason
    ?? plan?.fallback_reason
    ?? proxy.fallbackReason,
  );
  const connectMode = asString(
    record?.connect_mode
    ?? plan?.connect_mode
    ?? plan?.mode
    ?? proxy.connectMode,
  ) ?? defaultConnectMode;
  const remoteUrl = asString(record?.remote_url ?? route?.remote_url ?? plan?.remote_url ?? proxy.remoteUrl);
  const owner = asString(record?.owner ?? plan?.owner ?? proxy.backend);
  const cleanupCommand = asString(record?.cleanup_command)
    ?? (routeId ? `ssh_proxy node control stop-route ${routeId}` : undefined);

  return {
    routeId,
    owner,
    selectedTransport,
    connectMode,
    fallbackReason,
    remoteUrl,
    cleanupCommand,
    health: record?.health ?? route?.health ?? plan?.health,
    liveRoute: route,
  };
}

export function refreshSshProxyRouteState(
  state: SshProxyRouteState,
  routesJson: unknown,
): SshProxyRouteState {
  const liveRoute = findSshProxyLiveRoute(routesJson, state.routeId);
  if (!liveRoute) {
    return state;
  }

  const runtime = asRecord(liveRoute.runtime);
  const link = asRecord(liveRoute.link);
  const linkHealth = asRecord(link?.health);
  const selectedTransport = asString(
    runtime?.selected_transport
    ?? linkHealth?.selected_protocol
    ?? link?.selected_protocol,
  ) ?? state.selectedTransport;
  const fallbackReason = asString(
    runtime?.fallback_reason
    ?? liveRoute.fallback_reason
    ?? state.fallbackReason,
  );
  const connectMode = asString(runtime?.connect_mode ?? runtime?.mode) ?? state.connectMode;

  return {
    ...state,
    selectedTransport,
    connectMode,
    fallbackReason,
    health: linkHealth ?? link ?? state.health,
    liveRoute,
  };
}

export function findSshProxyLiveRoute(routesJson: unknown, routeId: string): Record<string, unknown> | undefined {
  const routes = asArray(asRecord(routesJson)?.routes);
  for (const route of routes) {
    const record = asRecord(route);
    if (asString(record?.id ?? record?.route_id) === routeId) {
      return record;
    }
  }
  return undefined;
}

function routeIdFrom(remoteUrl: string): string {
  return `vscode-remote-proxy-${hashTarget(remoteUrl)}`;
}

function hashTarget(value: string): string {
  let hash = 0;
  for (let index = 0; index < value.length; index += 1) {
    hash = (hash * 33 + value.charCodeAt(index)) >>> 0;
  }
  return hash.toString(16);
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
}

function asArray(value: unknown): readonly unknown[] {
  return Array.isArray(value) ? value : [];
}

function asString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}
