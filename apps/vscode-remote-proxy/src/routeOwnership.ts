export function shouldStopSshProxyRoute(routeId: string | undefined, ownsRoute: boolean): boolean {
  return Boolean(routeId && ownsRoute);
}

export function shouldReleaseOwnedLease(leaseMode: string, targetKey: string | undefined): boolean {
  return leaseMode === 'owner' && Boolean(targetKey);
}
