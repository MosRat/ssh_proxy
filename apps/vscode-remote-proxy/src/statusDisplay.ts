import { ProxyLeaseState } from './proxyLeaseRecord';
import { SshProxyKernelStatusSnapshot } from './sshProxyKernelStatus';
import { AppliedProxy, ForwardingBackendKind } from './types';

export interface RemoteProxyStatusLineInput {
  readonly status: string;
  readonly backend: ForwardingBackendKind;
  readonly remoteName: string;
  readonly remoteAuthority: string;
  readonly detectedHost: string | undefined;
  readonly detectedSource: string | undefined;
  readonly detectedConfidence: string;
  readonly forwardSshHost: string | undefined;
  readonly leaseMode: string;
  readonly leaseDescription: string;
  readonly restartBackoff: string;
  readonly proxy: AppliedProxy | undefined;
  readonly lease: ProxyLeaseState | undefined;
  readonly kernelStatus: SshProxyKernelStatusSnapshot | undefined;
  readonly lastError: string | undefined;
}

export function buildRemoteProxyStatusLines(input: RemoteProxyStatusLineInput): string[] {
  const proxy = input.proxy;
  return [
    `status: ${input.status}`,
    `backend: ${input.backend}`,
    `remote: ${input.remoteName} (${input.remoteAuthority})`,
    `ssh host: ${input.detectedHost ?? 'not resolved'}`,
    `ssh host source: ${input.detectedSource ?? 'none'}`,
    `ssh host confidence: ${input.detectedConfidence}`,
    `forward ssh host: ${input.forwardSshHost ?? 'not active'}`,
    `lease mode: ${input.leaseMode}`,
    `lease: ${input.leaseDescription}`,
    `restart backoff: ${input.restartBackoff}`,
    `local proxy: ${proxy ? `${proxy.local.url} from ${proxy.local.source}` : 'not active'}`,
    `remote proxy: ${proxy?.remoteUrl ?? 'not active'}`,
    `route id: ${proxy?.routeId ?? input.lease?.routeId ?? 'not active'}`,
    `route owner: ${proxy?.routeOwner ?? input.lease?.routeOwner ?? 'none'}`,
    `selected transport: ${proxy?.selectedTransport ?? input.lease?.selectedTransport ?? 'not active'}`,
    `connect mode: ${proxy?.connectMode ?? 'not active'}`,
    `fallback reason: ${proxy?.fallbackReason ?? 'none'}`,
    `daemon health: ${describeSshProxyDaemonHealth(input.backend, input.kernelStatus)}`,
    `route health: ${describeSshProxyRouteHealth(input.backend, input.kernelStatus)}`,
    `last error: ${input.lastError ?? 'none'}`,
  ];
}

export function describeSshProxyDaemonHealth(
  backend: ForwardingBackendKind,
  kernelStatus: SshProxyKernelStatusSnapshot | undefined,
): string {
  if (backend !== 'ssh_proxy') {
    return 'not applicable';
  }
  const status = asRecord(kernelStatus?.serviceStatus);
  if (!status) {
    return 'unknown';
  }

  const ok = asBool(status.ok);
  const daemon = asRecord(status.daemon);
  const daemonReachable = asBool(daemon?.reachable);
  const listeners = asRecord(asRecord(status.health)?.listeners);
  const controlOk = asBool(asRecord(listeners?.control)?.ok);
  const tlsOk = asBool(asRecord(listeners?.tls_tcp)?.ok);
  const plainOk = asBool(asRecord(listeners?.plain_tcp)?.ok);
  const quicConfigured = asBool(asRecord(listeners?.quic)?.configured);
  const error = asString(status.error);

  const parts = [
    `ok=${formatBool(ok)}`,
    `daemon=${formatBool(daemonReachable)}`,
    `control=${formatBool(controlOk)}`,
    `plain=${formatBool(plainOk)}`,
    `tls=${formatBool(tlsOk)}`,
    `quic=${formatBool(quicConfigured)}`,
  ];
  return error ? `${parts.join(' ')} error=${error}` : parts.join(' ');
}

export function describeSshProxyRouteHealth(
  backend: ForwardingBackendKind,
  kernelStatus: SshProxyKernelStatusSnapshot | undefined,
): string {
  if (backend !== 'ssh_proxy') {
    return 'not applicable';
  }
  const routeState = kernelStatus?.routeState;
  const health = asRecord(routeState?.health);
  if (!routeState || !health) {
    return 'unknown';
  }

  const selectedProtocol = asString(health.selected_protocol);
  const controlHealth = asString(health.control_health);
  const activeConnections = asNumber(health.active_connections);
  const activeStreams = asNumber(health.active_streams ?? health.active_channels);
  const openFailures = asNumber(health.open_failures ?? health.tcp_open_failures);
  const degradedReason = asString(health.degraded_reason);

  return [
    selectedProtocol ? `protocol=${selectedProtocol}` : undefined,
    controlHealth ? `control=${controlHealth}` : undefined,
    activeConnections !== undefined ? `connections=${activeConnections}` : undefined,
    activeStreams !== undefined ? `streams=${activeStreams}` : undefined,
    openFailures !== undefined ? `open_failures=${openFailures}` : undefined,
    degradedReason ? `degraded=${degradedReason}` : undefined,
  ].filter((part): part is string => Boolean(part)).join(' ') || 'available';
}

function formatBool(value: boolean | undefined): string {
  return value === undefined ? 'unknown' : String(value);
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
}

function asString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}

function asBool(value: unknown): boolean | undefined {
  return typeof value === 'boolean' ? value : undefined;
}

function asNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}
