import * as crypto from 'crypto';
import * as fs from 'fs/promises';
import * as os from 'os';
import * as path from 'path';
import * as vscode from 'vscode';
import { AppliedProxy } from './types';
import { buildProxyLeaseState, normalizeProxyLeaseState, ProxyLeaseState } from './proxyLeaseRecord';

export type { ProxyLeaseState } from './proxyLeaseRecord';

export interface ProxyLeaseManagerOptions {
  readonly leaseRoot?: string;
}

export interface ProxyStartLock {
  readonly targetKey: string;
  release(): Promise<void>;
}

interface StartLockState {
  readonly ownerId: string;
  readonly pid: number;
  readonly targetKey: string;
  readonly createdAt: number;
  readonly updatedAt: number;
}

export class ProxyLeaseManager {
  private readonly dir: string;

  public constructor(
    private readonly output: vscode.OutputChannel,
    private readonly ownerId: string,
    options: ProxyLeaseManagerOptions = {},
  ) {
    this.dir = options.leaseRoot ?? path.join(os.tmpdir(), 'vscode-remote-proxy', currentUserScopeKey());
  }

  public async read(targetKey: string): Promise<ProxyLeaseState | undefined> {
    try {
      const raw = await fs.readFile(this.getLeasePath(targetKey), 'utf8');
      return normalizeProxyLeaseState(JSON.parse(raw));
    } catch {
      return undefined;
    }
  }

  public async write(targetKey: string, sshHost: string, proxy: AppliedProxy): Promise<void> {
    await fs.mkdir(this.dir, { recursive: true });
    const previous = await this.read(targetKey);
    const state = buildProxyLeaseState({
      ownerId: this.ownerId,
      pid: process.pid,
      targetKey,
      sshHost,
      proxy,
      previous,
      now: Date.now(),
    });
    await this.writeJsonAtomic(this.getLeasePath(targetKey), state);
  }

  public async heartbeat(targetKey: string): Promise<void> {
    const state = await this.read(targetKey);
    if (!state || state.ownerId !== this.ownerId) {
      return;
    }
    await this.write(targetKey, state.sshHost, state.proxy);
  }

  public async release(targetKey: string): Promise<void> {
    const state = await this.read(targetKey);
    if (!state || state.ownerId !== this.ownerId) {
      return;
    }
    try {
      await fs.unlink(this.getLeasePath(targetKey));
    } catch {
      // Best effort cleanup.
    }
  }

  public isFresh(state: ProxyLeaseState, ttlMs: number): boolean {
    return Date.now() - state.updatedAt <= ttlMs;
  }

  public isOwnedByThisInstance(state: ProxyLeaseState): boolean {
    return state.ownerId === this.ownerId;
  }

  public isOwnerProcessAlive(state: ProxyLeaseState): boolean {
    return isProcessAlive(state.pid);
  }

  public async acquireStartLock(targetKey: string, timeoutMs: number, staleMs: number): Promise<ProxyStartLock> {
    await fs.mkdir(this.dir, { recursive: true });
    const normalizedTargetKey = normalizeTargetKey(targetKey);
    const lockPath = this.getLockPath(normalizedTargetKey);
    const deadline = Date.now() + Math.max(1, timeoutMs);
    let attempts = 0;

    while (true) {
      const now = Date.now();
      const state: StartLockState = {
        ownerId: this.ownerId,
        pid: process.pid,
        targetKey: normalizedTargetKey,
        createdAt: now,
        updatedAt: now,
      };

      try {
        const handle = await fs.open(lockPath, 'wx');
        try {
          await handle.writeFile(JSON.stringify(state, null, 2), 'utf8');
        } finally {
          await handle.close();
        }
        return {
          targetKey: normalizedTargetKey,
          release: async () => {
            await this.releaseStartLock(lockPath, state);
          },
        };
      } catch (error) {
        if (!isAlreadyExists(error)) {
          throw error;
        }

        const existing = await this.readStartLock(lockPath);
        const stale = !existing
          || Date.now() - existing.updatedAt > Math.max(1, staleMs)
          || !isProcessAlive(existing.pid);
        if (stale) {
          await this.releaseStartLock(lockPath, existing);
          continue;
        }

        if (Date.now() >= deadline) {
          throw new Error(`another VS Code window is still starting Remote Proxy for ${normalizedTargetKey}`);
        }

        attempts += 1;
        await sleep(Math.min(1000, 100 + attempts * 100));
      }
    }
  }

  public describe(state: ProxyLeaseState | undefined): string {
    if (!state) {
      return 'none';
    }
    const ageSeconds = Math.max(0, Math.round((Date.now() - state.updatedAt) / 1000));
    return `${state.ownerId === this.ownerId ? 'owned' : 'shared'} owner=${state.ownerId.slice(0, 8)} pid=${state.pid} age=${ageSeconds}s target=${state.targetKey} backend=${state.backend}${state.routeId ? ` route=${state.routeId}` : ''}${state.selectedTransport ? ` transport=${state.selectedTransport}` : ''}`;
  }

  public getStableTargetKey(sshHost: string, detectedTargetKey: string | undefined): string {
    return normalizeTargetKey(detectedTargetKey || sshHost);
  }

  private getLeasePath(targetKey: string): string {
    const hash = crypto.createHash('sha256').update(normalizeTargetKey(targetKey)).digest('hex').slice(0, 32);
    return path.join(this.dir, `${hash}.json`);
  }

  private getLockPath(targetKey: string): string {
    const hash = crypto.createHash('sha256').update(normalizeTargetKey(targetKey)).digest('hex').slice(0, 32);
    return path.join(this.dir, `${hash}.lock`);
  }

  private async writeJsonAtomic(filePath: string, value: unknown): Promise<void> {
    const tempPath = `${filePath}.${this.ownerId}.${process.pid}.tmp`;
    await fs.writeFile(tempPath, JSON.stringify(value, null, 2), 'utf8');
    await fs.rename(tempPath, filePath);
  }

  private async readStartLock(lockPath: string): Promise<StartLockState | undefined> {
    try {
      const raw = await fs.readFile(lockPath, 'utf8');
      const parsed = JSON.parse(raw);
      if (!parsed || typeof parsed !== 'object') {
        return undefined;
      }
      const record = parsed as Record<string, unknown>;
      if (
        typeof record.ownerId !== 'string'
        || typeof record.pid !== 'number'
        || typeof record.targetKey !== 'string'
        || typeof record.createdAt !== 'number'
        || typeof record.updatedAt !== 'number'
      ) {
        return undefined;
      }
      return {
        ownerId: record.ownerId,
        pid: record.pid,
        targetKey: record.targetKey,
        createdAt: record.createdAt,
        updatedAt: record.updatedAt,
      };
    } catch {
      return undefined;
    }
  }

  private async releaseStartLock(lockPath: string, expected?: StartLockState): Promise<void> {
    try {
      if (expected) {
        const current = await this.readStartLock(lockPath);
        if (!current || current.ownerId !== expected.ownerId || current.pid !== expected.pid) {
          return;
        }
      }
      await fs.unlink(lockPath);
    } catch {
      // Best effort cleanup.
    }
  }
}

export function createOwnerId(): string {
  return crypto.randomUUID();
}

function normalizeTargetKey(value: string): string {
    return value.trim().toLowerCase();
}

function currentUserScopeKey(): string {
  const user = os.userInfo();
  const uid = user.uid >= 0 ? String(user.uid) : user.username;
  const host = os.hostname();
  return crypto.createHash('sha256').update(`${uid}@${host}`).digest('hex').slice(0, 16);
}

function isProcessAlive(pid: number): boolean {
  if (!Number.isInteger(pid) || pid <= 0) {
    return false;
  }
  if (pid === process.pid) {
    return true;
  }
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if (
      typeof error === 'object'
      && error !== null
      && 'code' in error
      && (error as NodeJS.ErrnoException).code === 'EPERM'
    ) {
      return true;
    }
    return false;
  }
}

function isAlreadyExists(error: unknown): boolean {
  return typeof error === 'object'
    && error !== null
    && 'code' in error
    && (error as NodeJS.ErrnoException).code === 'EEXIST';
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
