import type { AppliedProxy } from './types';

export const REMOTE_PROXY_DIAGNOSTICS_HEADER = 'Remote Proxy diagnostics';
export const REMOTE_PROXY_DIAGNOSTICS_SKIP_LINE = 'diagnose: forwarder is not running, so remote port verification was skipped.';

export function shouldVerifyRemoteForward(
  status: string,
  proxy: AppliedProxy | undefined,
  sshHost: string | undefined,
): proxy is AppliedProxy {
  return status === 'running' && Boolean(proxy && sshHost);
}

export function remoteForwardReachableLine(proxy: AppliedProxy): string {
  return `diagnose: remote forwarded port is reachable at ${proxy.remoteUrl}`;
}

export function remoteForwardFailureLine(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return `diagnose: remote forwarded port check failed: ${message}`;
}
