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
import { AppliedProxy, RemoteProxyConfig } from './types';

export class SshProxyDaemonRejectedError extends Error {
  public readonly payload: unknown;
  public readonly blocker: string | undefined;
  public readonly nextAction: string | undefined;
  public readonly requiresDaemon: boolean;
  public readonly requiresElevation: boolean;
  public readonly retryAfterMs: number | undefined;

  public constructor(payload: unknown) {
    const record = asRecord(payload);
    const job = asRecord(record?.job);
    const blocker = asString(record?.blocker) ?? asString(job?.phase) ?? 'daemon_rejected';
    const detail = asString(record?.error) ?? asString(job?.message) ?? asString(record?.last_error);
    super(`ssh_proxy daemon blocked (${blocker})${detail ? `: ${detail}` : ''}`);
    this.name = 'SshProxyDaemonRejectedError';
    this.payload = payload;
    this.blocker = blocker;
    this.nextAction = asString(record?.next_action) ?? asString(job?.next_action);
    this.requiresDaemon = record?.requires_daemon === true;
    this.requiresElevation = record?.requires_elevation === true;
    const retryAfterMs = record?.retry_after_ms;
    this.retryAfterMs = typeof retryAfterMs === 'number' ? retryAfterMs : undefined;
  }

  public get userMessage(): string {
    if (this.blocker === 'daemon_unavailable') {
      return 'ssh_proxy daemon is not installed or not running';
    }
    if (this.blocker === 'daemon_pipe_access_denied') {
      return 'ssh_proxy daemon is running but its control pipe denied this user; reinstall or restart the daemon to repair permissions';
    }
    return this.message;
  }
}

export function isSshProxyDaemonRejectedError(error: unknown): error is SshProxyDaemonRejectedError {
  return error instanceof SshProxyDaemonRejectedError;
}

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

  public async startAndWait(config: RemoteProxyConfig, sshHost: string, proxy: AppliedProxy, _waitMs: number): Promise<void> {
    await this.start(config, sshHost, proxy);
  }

  public async start(config: RemoteProxyConfig, sshHost: string, proxy: AppliedProxy): Promise<void> {
    this.statusValue = 'starting';
    this.currentProxy = proxy;
    this.currentSshHostValue = sshHost;
    this.lastErrorValue = undefined;
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
        sshTarget: proxy.sshTarget,
        workspacePaths: remoteWorkspacePaths(),
        serverDir: serverDirName(),
        noProxy: config.noProxy,
        proxySupport: config.proxySupport,
        applyRemoteMachineSettings: config.applyRemoteMachineSettings,
        applyTerminalEnv: config.applyTerminalEnv,
        applyServerEnvSetup: config.applyServerEnvSetup,
        applyGitConfig: config.applyGitConfig,
        applyGitGlobalConfig: config.applyGitGlobalConfig,
        applyGitWorkspaceConfig: config.applyGitWorkspaceConfig,
        applyGitForceOverride: config.applyGitForceOverride,
        applyRemoteStatusFile: config.applyRemoteStatusFile,
        verifyRemotePort: config.verifyAfterStart,
      });
      const record = asRecord(started);
      if (record?.ok === false) {
        throw new SshProxyDaemonRejectedError(started);
      }
      const routeState = createSshProxyRouteState(started, proxy, config.sshProxyConnectMode);
      this.setSnapshot({ sessionStart: started, routeState });
      await this.waitForProxySessionReadiness(
        cli,
        proxy.workspaceId ?? sshHost,
        sshHost,
        routeState,
      );
      this.applyRouteState(proxy, this.snapshot.routeState ?? routeState);

      this.statusValue = 'running';
      this.lastErrorValue = undefined;
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
    if (routeId || this.currentProxy?.workspaceId || this.currentSshHostValue) {
      try {
        const cli = this.cliForCurrent();
        if (cli) {
          const stopped = await cli.downJson({
            routeId,
            workspace: this.currentProxy?.workspaceId,
            target: this.currentSshHostValue,
          });
          this.setSnapshot({ sessionStop: stopped });
          this.output.appendLine(`ssh_proxy down: ${prettyJson(stopped)}`);
        }
      } catch (error) {
        this.lastErrorValue = error instanceof Error ? error.message : String(error);
        this.output.appendLine(`ssh_proxy down failed: ${this.lastErrorValue}`);
      }
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
      this.snapshot = emptySshProxyKernelStatusSnapshot();
      this.statusValue = 'stopped';
      this.changeEmitter.fire();
    }
  }

  public dispose(): void {
    void this.stop();
    this.changeEmitter.dispose();
  }

  public async refreshStatus(): Promise<void> {
    const cli = this.cliForCurrent();
    const workspace = this.currentProxy?.workspaceId ?? this.currentSshHostValue;
    if (!cli || !workspace || !this.currentSshHostValue) {
      return;
    }
    const status = await cli.vscodeStatusJson({ workspace, target: this.currentSshHostValue }, { logCommand: false });
    this.setSnapshot({ daemonStatus: status, lastRefreshAt: Date.now() });
    const record = asRecord(status);
    const job = asRecord(record?.job);
    const route = asRecord(record?.route);
    if (record?.remote_url || route) {
      const currentRoute = this.snapshot.routeState;
      this.snapshot = {
        ...this.snapshot,
        routeState: currentRoute ? {
          ...currentRoute,
          remoteUrl: asString(record?.remote_url) ?? asString(route?.remote_url) ?? currentRoute.remoteUrl,
          health: job ?? route ?? currentRoute.health,
          liveRoute: route,
        } : undefined,
      };
    }
    const state = asString(job?.state) ?? asString(record?.health);
    if (state === 'failed' || state === 'cancelled') {
      this.statusValue = 'failed';
      this.lastErrorValue = asString(job?.last_error) ?? asString(record?.last_error) ?? 'ssh_proxy daemon job failed';
    } else if (state === 'waiting_retry') {
      this.statusValue = 'starting';
      this.lastErrorValue = asString(job?.phase) === 'verify_remote_port'
        ? 'ssh_proxy daemon is waiting for remote handoff'
        : undefined;
    } else if (state === 'healthy' || record?.health === 'healthy') {
      this.statusValue = 'running';
      this.lastErrorValue = undefined;
    }
    this.changeEmitter.fire();
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
    const deadline = Date.now() + 120_000;
    let lastPhase = 'queued';
    let lastError: string | undefined;
    while (Date.now() <= deadline) {
      const status = await cli.vscodeStatusJson({ workspace, target }, { logCommand: false });
      this.setSnapshot({ daemonStatus: status, lastRefreshAt: Date.now() });
      const record = asRecord(status);
      const job = asRecord(record?.job);
      const route = asRecord(record?.route);
      const state = asString(job?.state) ?? asString(record?.health) ?? 'unknown';
      lastPhase = asString(job?.phase) ?? lastPhase;
      lastError = asString(job?.last_error) ?? asString(record?.last_error) ?? lastError;
      if (state === 'waiting_retry') {
        this.statusValue = 'starting';
        if (lastPhase === 'verify_remote_port') {
          this.lastErrorValue = 'ssh_proxy daemon is waiting for remote handoff';
        }
        this.changeEmitter.fire();
      }
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

function serverDirName(): string {
  return vscode.env.appName.toLowerCase().includes('insider') ? '.vscode-server-insiders' : '.vscode-server';
}

function remoteWorkspacePaths(): string[] {
  const paths = new Set<string>();
  for (const folder of vscode.workspace.workspaceFolders ?? []) {
    if (folder.uri.scheme === 'vscode-remote' && folder.uri.path) {
      paths.add(folder.uri.path);
    }
  }
  return [...paths];
}
