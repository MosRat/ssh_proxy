import { RemoteSetupMode } from './types';

export type RemoteCommandRunnerKind = 'openssh' | 'ssh_proxy_host_exec';

export interface RemoteSetupFallbackRecord {
  readonly label: string;
  readonly preferred: RemoteCommandRunnerKind;
  readonly fallback: RemoteCommandRunnerKind;
  readonly reason: string;
  readonly at: number;
}

export function preferredRemoteSetupRunnerKind(mode: RemoteSetupMode): RemoteCommandRunnerKind {
  return mode === 'openssh' ? 'openssh' : 'ssh_proxy_host_exec';
}

export function shouldFallbackRemoteSetup(
  mode: RemoteSetupMode,
  preferred: RemoteCommandRunnerKind,
): boolean {
  return mode === 'auto' && preferred === 'ssh_proxy_host_exec';
}

export function createRemoteSetupFallbackRecord(
  label: string,
  preferred: RemoteCommandRunnerKind,
  fallback: RemoteCommandRunnerKind,
  error: unknown,
  at = Date.now(),
): RemoteSetupFallbackRecord {
  return {
    label,
    preferred,
    fallback,
    reason: error instanceof Error ? error.message : String(error),
    at,
  };
}

export function formatRemoteSetupFallbackReason(record: RemoteSetupFallbackRecord): string {
  return `${record.label}: ${record.preferred} failed; fallback=${record.fallback}; reason=${record.reason}`;
}
