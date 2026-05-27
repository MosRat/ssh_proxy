export interface RestartBackoffDecision {
  readonly canRestart: boolean;
  readonly restartFailures: number;
  readonly nextRestartAt: number;
  readonly waitSeconds?: number;
}

export function leaseHeartbeatIntervalMs(singletonLeaseTtlSeconds: number, healthCheckIntervalSeconds: number): number {
  return Math.min(
    Math.max(1000, singletonLeaseTtlSeconds * 500),
    Math.max(1000, healthCheckIntervalSeconds * 1000),
  );
}

export function healthCheckIntervalMs(healthCheckIntervalSeconds: number): number {
  return Math.max(1000, healthCheckIntervalSeconds * 1000);
}

export function shouldRunTimedCheck(now: number, lastRunAt: number, intervalMs: number): boolean {
  return now - lastRunAt >= intervalMs;
}

export function decideRestartBackoff(
  now: number,
  nextRestartAt: number,
  restartFailures: number,
  restartBackoffMaxSeconds: number,
): RestartBackoffDecision {
  if (nextRestartAt > now) {
    return {
      canRestart: false,
      restartFailures,
      nextRestartAt,
      waitSeconds: Math.ceil((nextRestartAt - now) / 1000),
    };
  }

  const nextFailures = Math.max(0, restartFailures) + 1;
  const delaySeconds = Math.min(restartBackoffMaxSeconds, 5 * 2 ** Math.max(0, nextFailures - 1));
  return {
    canRestart: true,
    restartFailures: nextFailures,
    nextRestartAt: now + delaySeconds * 1000,
  };
}
