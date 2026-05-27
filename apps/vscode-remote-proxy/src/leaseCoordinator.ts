import * as vscode from 'vscode';
import { createOwnerId, ProxyLeaseManager, ProxyLeaseState, ProxyStartLock } from './proxyLease';
import { shouldReleaseOwnedLease } from './routeOwnership';
import { AppliedProxy } from './types';

export type LeaseMode = 'none' | 'owner' | 'shared';

export class LeaseCoordinator {
  private readonly manager: ProxyLeaseManager;
  private modeValue: LeaseMode = 'none';
  private targetKeyValue: string | undefined;
  private lastHeartbeatAtValue = 0;

  public constructor(output: vscode.OutputChannel) {
    this.manager = new ProxyLeaseManager(output, createOwnerId());
  }

  public get mode(): LeaseMode {
    return this.modeValue;
  }

  public get targetKey(): string | undefined {
    return this.targetKeyValue;
  }

  public get lastHeartbeatAt(): number {
    return this.lastHeartbeatAtValue;
  }

  public resetHeartbeat(): void {
    this.lastHeartbeatAtValue = 0;
  }

  public markHeartbeat(now: number): void {
    this.lastHeartbeatAtValue = now;
  }

  public markOwner(targetKey: string): void {
    this.modeValue = 'owner';
    this.targetKeyValue = targetKey;
  }

  public markShared(targetKey: string): void {
    this.modeValue = 'shared';
    this.targetKeyValue = targetKey;
  }

  public markFromLease(targetKey: string, lease: ProxyLeaseState): LeaseMode {
    const mode = this.manager.isOwnedByThisInstance(lease) ? 'owner' : 'shared';
    this.modeValue = mode;
    this.targetKeyValue = targetKey;
    return mode;
  }

  public clear(): void {
    this.modeValue = 'none';
    this.targetKeyValue = undefined;
    this.lastHeartbeatAtValue = 0;
  }

  public getStableTargetKey(sshHost: string, detectedTargetKey: string | undefined): string {
    return this.manager.getStableTargetKey(sshHost, detectedTargetKey);
  }

  public acquireStartLock(targetKey: string, timeoutMs: number, staleMs: number): Promise<ProxyStartLock> {
    return this.manager.acquireStartLock(targetKey, timeoutMs, staleMs);
  }

  public read(targetKey: string): Promise<ProxyLeaseState | undefined> {
    return this.manager.read(targetKey);
  }

  public readCurrent(): Promise<ProxyLeaseState | undefined> {
    return this.targetKeyValue ? this.manager.read(this.targetKeyValue) : Promise.resolve(undefined);
  }

  public write(targetKey: string, sshHost: string, proxy: AppliedProxy): Promise<void> {
    return this.manager.write(targetKey, sshHost, proxy);
  }

  public heartbeatCurrent(): Promise<void> {
    return this.targetKeyValue ? this.manager.heartbeat(this.targetKeyValue) : Promise.resolve();
  }

  public releaseOwned(): Promise<void> {
    if (shouldReleaseOwnedLease(this.modeValue, this.targetKeyValue) && this.targetKeyValue) {
      return this.manager.release(this.targetKeyValue);
    }
    return Promise.resolve();
  }

  public describe(lease: ProxyLeaseState | undefined): string {
    return this.manager.describe(lease);
  }

  public isFresh(lease: ProxyLeaseState, ttlMs: number): boolean {
    return this.manager.isFresh(lease, ttlMs);
  }

  public isOwnedByThisInstance(lease: ProxyLeaseState): boolean {
    return this.manager.isOwnedByThisInstance(lease);
  }

  public isOwnerProcessAlive(lease: ProxyLeaseState): boolean {
    return this.manager.isOwnerProcessAlive(lease);
  }
}
