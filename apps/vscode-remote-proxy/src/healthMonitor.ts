export function healthCheckIntervalMs(healthCheckIntervalSeconds: number): number {
  return Math.max(1000, healthCheckIntervalSeconds * 1000);
}

export function shouldRunTimedCheck(now: number, lastRunAt: number, intervalMs: number): boolean {
  return now - lastRunAt >= intervalMs;
}
