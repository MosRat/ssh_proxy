export interface ForwardFailureLike {
  readonly deterministic?: boolean;
  readonly message?: string;
}

export interface HealthFailureDecision {
  readonly failures: number;
  readonly threshold: number;
  readonly shouldRestart: boolean;
}

const DEFAULT_HEALTH_FAILURE_THRESHOLD = 2;
const PORT_FAILURE_PATTERN = /remote port forwarding failed|cannot listen to port|address already in use|address already allocated|eaddrinuse|bind:.*in use|listen .* failed/i;

export function clampHealthFailureThreshold(value: number): number {
  if (!Number.isFinite(value)) {
    return DEFAULT_HEALTH_FAILURE_THRESHOLD;
  }
  return Math.min(10, Math.max(1, Math.round(value)));
}

export function recordHealthCheckFailure(previousFailures: number, configuredThreshold: number): HealthFailureDecision {
  const threshold = clampHealthFailureThreshold(configuredThreshold);
  const failures = Math.max(0, previousFailures) + 1;
  return {
    failures,
    threshold,
    shouldRestart: failures >= threshold,
  };
}

export function shouldRetryForwardAttempt(error: unknown): boolean {
  const failure = error as ForwardFailureLike;
  if (failure?.deterministic === true) {
    return true;
  }
  const message = error instanceof Error ? error.message : String(error);
  return isDeterministicPortFailureMessage(message);
}

export function isDeterministicPortFailureMessage(message: string): boolean {
  return PORT_FAILURE_PATTERN.test(message);
}
