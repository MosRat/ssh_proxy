import * as vscode from 'vscode';
import { ForwardingBackend, ForwardingBackendStatus } from './forwardingBackend';
import { findAvailableSshProxyCli, SshProxyCli } from './sshProxyCli';
import {
  createSshProxyRouteState,
  emptySshProxyKernelStatusSnapshot,
  SshProxyKernelStatusSnapshot,
  SshProxyRouteState,
} from './sshProxyKernelStatus';
import { resolveSshProxyExecutableCandidates, sshProxyUnavailableCandidatesMessage } from './sshProxyDiscovery';
import { shouldStopSshProxyRoute } from './routeOwnership';
import { AppliedProxy, RemoteProxyConfig } from './types';

export class SshProxyKernelBackend implements ForwardingBackend {
  private readonly changeEmitter = new vscode.EventEmitter<void>();
  private childRouteId: string | undefined;
  private currentCliKey: string | undefined;
  private currentProxy: AppliedProxy | undefined;
  private currentSshHostValue: string | undefined;
  private statusValue: ForwardingBackendStatus = 'stopped';
  private lastErrorValue: string | undefined;
  private currentSelectedTransport: string | undefined;
  private currentFallbackReason: string | undefined;
  private currentConnectMode: string | undefined;
  private ownsCurrentRoute = false;
  private snapshot: SshProxyKernelStatusSnapshot = emptySshProxyKernelStatusSnapshot();
  private currentCli: SshProxyCli | undefined;

  public readonly onDidChange = this.changeEmitter.event;

  public constructor(
    private readonly output: vscode.OutputChannel,
    private readonly extensionPath?: string,
  ) {
  }

  public get status(): ForwardingBackendStatus {
    return this.statusValue;
  }

  public get lastError(): string | undefined {
    return this.lastErrorValue;
  }

  public get appliedProxy(): AppliedProxy | undefined {
    return this.currentProxy;
  }

  public get currentSshHost(): string | undefined {
    return this.currentSshHostValue;
  }

  public get kernelStatus(): SshProxyKernelStatusSnapshot {
    return this.snapshot;
  }

  public fail(message: string): void {
    this.lastErrorValue = message;
    this.statusValue = 'failed';
    this.changeEmitter.fire();
  }

  public adoptShared(sshHost: string, proxy: AppliedProxy): void {
    this.currentSshHostValue = sshHost;
    this.currentProxy = proxy;
    this.childRouteId = proxy.routeId;
    this.currentSelectedTransport = proxy.selectedTransport;
    this.currentFallbackReason = proxy.fallbackReason;
    this.currentConnectMode = proxy.connectMode;
    this.ownsCurrentRoute = false;
    this.snapshot = {
      ...this.snapshot,
      routeState: proxy.routeId ? {
        routeId: proxy.routeId,
        owner: proxy.routeOwner ?? proxy.backend,
        selectedTransport: proxy.selectedTransport,
        connectMode: proxy.connectMode,
        fallbackReason: proxy.fallbackReason,
        remoteUrl: proxy.remoteUrl,
        cleanupCommand: proxy.cleanupCommand,
        health: undefined,
        liveRoute: undefined,
      } : undefined,
    };
    this.lastErrorValue = undefined;
    this.statusValue = 'running';
    this.changeEmitter.fire();
  }

  public async startAndWait(config: RemoteProxyConfig, sshHost: string, proxy: AppliedProxy, _waitMs: number): Promise<void> {
    await this.start(config, sshHost, proxy);
  }

  public async start(config: RemoteProxyConfig, sshHost: string, proxy: AppliedProxy): Promise<void> {
    this.statusValue = 'starting';
    this.currentProxy = proxy;
    this.currentSshHostValue = sshHost;
    this.lastErrorValue = undefined;
    this.ownsCurrentRoute = false;
    this.snapshot = emptySshProxyKernelStatusSnapshot();
    this.changeEmitter.fire();

    try {
      const cli = await this.availableCli(config);

      const started = await cli.vscodeUpJson({
        target: sshHost,
        workspace: proxy.workspaceId ?? sshHost,
        localProxy: proxy.local.url,
        remoteBind: proxy.remoteBindHost,
        remotePort: proxy.remotePort,
        connectMode: config.sshProxyConnectMode,
      });
      const record = asRecord(started);
      if (record?.ok === false) {
        throw new Error(`ssh_proxy daemon rejected the proxy session: ${prettyJson(started)}`);
      }
      const routeState = createSshProxyRouteState(started, proxy, config.sshProxyConnectMode);
      this.setSnapshot({ routeStart: started, routeState });
      await this.waitForProxySessionReadiness(
        cli,
        proxy.workspaceId ?? sshHost,
        sshHost,
        routeState,
      );
      this.applyRouteState(proxy, this.snapshot.routeState ?? routeState);

      this.statusValue = 'running';
      this.changeEmitter.fire();
      if (record?.health) {
        this.output.appendLine(`ssh_proxy route health: ${prettyJson(record.health)}`);
      }
    } catch (error) {
      this.lastErrorValue = error instanceof Error ? error.message : String(error);
      this.statusValue = 'failed';
      this.changeEmitter.fire();
      throw error;
    }
  }

  public async stop(clearIntent = true): Promise<void> {
    const routeId = this.childRouteId;
    if (shouldStopSshProxyRoute(routeId, this.ownsCurrentRoute) && routeId) {
      try {
        const cli = this.cliForCurrent();
        if (cli) {
          const stopped = await cli.downJson({
            routeId,
            workspace: this.currentProxy?.workspaceId,
            target: this.currentSshHostValue,
          });
          this.setSnapshot({ routeStop: stopped });
          this.output.appendLine(`ssh_proxy down: ${prettyJson(stopped)}`);
        }
      } catch (error) {
        this.lastErrorValue = error instanceof Error ? error.message : String(error);
        this.output.appendLine(`ssh_proxy down failed: ${this.lastErrorValue}`);
      }
    } else if (routeId) {
      this.output.appendLine(`ssh_proxy route ${routeId} is shared or not owned by this window; detaching without stop-route`);
    }

    if (clearIntent) {
      this.currentProxy = undefined;
      this.currentSshHostValue = undefined;
      this.childRouteId = undefined;
      this.currentCliKey = undefined;
      this.currentCli = undefined;
      this.currentSelectedTransport = undefined;
      this.currentFallbackReason = undefined;
      this.currentConnectMode = undefined;
      this.ownsCurrentRoute = false;
      this.snapshot = emptySshProxyKernelStatusSnapshot();
      this.statusValue = 'stopped';
      this.changeEmitter.fire();
    }
  }

  public dispose(): void {
    void this.stop();
    this.changeEmitter.dispose();
  }

  private cliForCurrent(): SshProxyCli | undefined {
    return this.currentCliKey ? this.currentCli : undefined;
  }

  private async availableCli(config: RemoteProxyConfig): Promise<SshProxyCli> {
    const resolved = await findAvailableSshProxyCli(
      config.sshProxyExecutable,
      this.output,
      { extensionPath: this.extensionPath },
    );
    if (resolved) {
      this.currentCliKey = resolved.discovery.executable;
      this.currentCli = resolved.cli;
      return resolved.cli;
    }
    throw new Error(sshProxyUnavailableCandidatesMessage(resolveSshProxyExecutableCandidates(
      config.sshProxyExecutable,
      { extensionPath: this.extensionPath },
    )));
  }

  private applyRouteState(proxy: AppliedProxy, routeState: SshProxyRouteState): void {
    this.childRouteId = routeState.routeId;
    this.ownsCurrentRoute = true;
    this.currentSelectedTransport = routeState.selectedTransport;
    this.currentFallbackReason = routeState.fallbackReason;
    this.currentConnectMode = routeState.connectMode;
    this.currentProxy = {
      ...proxy,
      remoteUrl: routeState.remoteUrl ?? proxy.remoteUrl,
      routeId: routeState.routeId,
      routeOwner: routeState.owner,
      selectedTransport: routeState.selectedTransport,
      connectMode: routeState.connectMode,
      fallbackReason: routeState.fallbackReason,
      backend: 'ssh_proxy',
      cleanupCommand: routeState.cleanupCommand,
    };
  }

  private setSnapshot(update: Partial<SshProxyKernelStatusSnapshot>): void {
    this.snapshot = {
      ...this.snapshot,
      ...update,
    };
  }

  private async waitForProxySessionReadiness(
    cli: SshProxyCli,
    workspace: string,
    target: string,
    initialRouteState: SshProxyRouteState,
  ): Promise<void> {
    const deadline = Date.now() + 60_000;
    let lastPhase = 'queued';
    let lastError: string | undefined;
    while (Date.now() <= deadline) {
      const status = await cli.vscodeStatusJson({ workspace, target });
      this.setSnapshot({ serviceStatus: status, lastRefreshAt: Date.now() });
      const record = asRecord(status);
      const job = asRecord(record?.job);
      const route = asRecord(record?.route);
      const state = asString(job?.state) ?? asString(record?.health) ?? 'unknown';
      lastPhase = asString(job?.phase) ?? lastPhase;
      lastError = asString(job?.last_error) ?? asString(record?.last_error) ?? lastError;
      if (route || record?.remote_url) {
        this.setSnapshot({
          routeState: {
            ...initialRouteState,
            remoteUrl: asString(record?.remote_url) ?? asString(route?.remote_url) ?? initialRouteState.remoteUrl,
            health: job ?? route ?? initialRouteState.health,
            liveRoute: route,
          },
        });
      }
      if (state === 'failed' || state === 'cancelled') {
        throw new Error(`ssh_proxy daemon job failed in ${lastPhase}: ${lastError ?? 'unknown error'}`);
      }
      if (state === 'healthy' || record?.health === 'healthy') {
        return;
      }
      await sleep(500);
    }
    throw new Error(`ssh_proxy daemon job did not become healthy; last phase=${lastPhase}${lastError ? ` error=${lastError}` : ''}`);
  }
}

function prettyJson(value: unknown): string {
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
}

function asString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
