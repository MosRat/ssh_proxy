import { makeRemoteProxyUrl } from './proxyDetection';
import { ProxyLeaseState } from './proxyLease';
import { RemoteProxyStatusFile } from './remoteStatusFile';
import { AppliedProxy, ForwardingBackendKind, LocalProxy, RemoteProxyConfig } from './types';

export interface RemotePortCandidateInput {
  readonly config: RemoteProxyConfig;
  readonly local: LocalProxy;
  readonly currentProxy?: AppliedProxy;
  readonly preferredPort?: number;
  readonly preferredBindHost?: string;
  readonly lease?: ProxyLeaseState;
  readonly remoteStatus?: RemoteProxyStatusFile;
}

export function buildRemotePortCandidates(input: RemotePortCandidateInput): number[] {
  const ports: number[] = [];
  addPort(ports, sameBind(input.currentProxy?.remoteBindHost, input.config.remoteBindHost) ? input.currentProxy?.remotePort : undefined);
  addPort(ports, sameBind(input.preferredBindHost, input.config.remoteBindHost) ? input.preferredPort : undefined);
  addPort(ports, sameBind(input.lease?.proxy.remoteBindHost, input.config.remoteBindHost) ? input.lease?.proxy.remotePort : undefined);
  if (remoteStatusMatchesCurrentProxy(input.remoteStatus, input.config, input.local)) {
    addPort(ports, input.remoteStatus?.port);
  }

  const count = input.config.remoteAutoPickPort ? input.config.remotePortRangeSize : 1;
  for (let offset = 0; offset < count; offset += 1) {
    addPort(ports, input.config.remotePort + offset);
  }
  return ports;
}

export function remoteStatusMatchesCurrentProxy(
  status: RemoteProxyStatusFile | undefined,
  config: RemoteProxyConfig,
  local: LocalProxy,
): boolean {
  if (!status || !isValidPort(status.port)) {
    return false;
  }
  if (status.bindHost && status.bindHost !== config.remoteBindHost) {
    return false;
  }
  if (status.localProxyUrl && status.localProxyUrl !== local.url) {
    return false;
  }
  return true;
}

export function appliedProxyFromRemoteStatus(
  status: RemoteProxyStatusFile,
  config: RemoteProxyConfig,
  local: LocalProxy,
): AppliedProxy | undefined {
  if (!remoteStatusMatchesCurrentProxy(status, config, local) || !status.port) {
    return undefined;
  }
  const remoteBindHost = status.bindHost ?? config.remoteBindHost;
  return {
    local,
    remoteUrl: status.proxyUrl ?? makeRemoteProxyUrl(local, remoteBindHost, status.port),
    remotePort: status.port,
    remoteBindHost,
    backend: leaseBackend(status.backend),
    routeId: status.routeId,
    routeOwner: status.routeOwner,
    selectedTransport: status.selectedTransport,
    connectMode: status.connectMode,
    fallbackReason: status.fallbackReason,
  };
}

function addPort(ports: number[], port: number | undefined): void {
  if (!isValidPort(port) || ports.includes(port)) {
    return;
  }
  ports.push(port);
}

function isValidPort(port: number | undefined): port is number {
  return typeof port === 'number' && Number.isInteger(port) && port >= 1 && port <= 65535;
}

function sameBind(candidate: string | undefined, expected: string): boolean {
  return !candidate || candidate === expected;
}

function leaseBackend(value: string | undefined): Exclude<ForwardingBackendKind, 'auto'> | undefined {
  return value === 'ssh_proxy' || value === 'openssh' ? value : undefined;
}
