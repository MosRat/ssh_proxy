export class KernelRecoveryCoordinator {
  private readonly blockedReasons = new Map<string, string>();

  public block(key: string, reason: string): void {
    this.blockedReasons.set(key, reason);
  }

  public isBlocked(key: string): boolean {
    return this.blockedReasons.has(key);
  }

  public reason(key: string): string | undefined {
    return this.blockedReasons.get(key);
  }

  public recordServiceFailure(message: string): void {
    if (isPermissionDeniedMessage(message)) {
      this.block('persistent-service-install', message);
    }
  }
}

export function isPermissionDeniedMessage(message: string): boolean {
  return /access is denied|permission denied|requires administrator|requires root|privilege|elevation/i.test(message);
}

