import * as vscode from 'vscode';
import { ForwardingBackend, ForwardingBackendStatus } from './forwardingBackend';
import { findAvailableSshProxyCli, SshProxyCli } from './sshProxyCli';
import {
  createSshProxyRouteState,
  emptySshProxyKernelStatusSnapshot,
  isSshProxyOk,
  refreshSshProxyRouteState,
  SshProxyKernelStatusSnapshot,
  SshProxyRouteState,
} from './sshProxyKernelStatus';
import { resolveSshProxyExecutableCandidates, sshProxyUnavailableCandidatesMessage } from './sshProxyDiscovery';
import { SshProxyControlConnection } from './sshProxyCliUtils';
import { isPermissionDeniedMessage, KernelRecoveryCoordinator } from './kernelRecoveryCoordinator';
import { shouldStopSshProxyRoute } from './routeOwnership';
import {
  shutdownSshProxySessionDaemon,
  startSshProxySessionDaemon,
  SshProxySessionDaemon,
} from './sshProxySessionDaemon';
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
  private currentControl: SshProxyControlConnection | undefined;
  private sessionDaemon: SshProxySessionDaemon | undefined;
  private readonly recovery = new KernelRecoveryCoordinator();

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
      this.currentControl = undefined;

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
      await this.stopSessionDaemonIfUnhealthy();
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
          const stopped = await cli.stopRouteJson(routeId, this.currentControl);
          this.setSnapshot({ routeStop: stopped });
          this.output.appendLine(`ssh_proxy stop-route: ${prettyJson(stopped)}`);
        }
      } catch (error) {
        this.lastErrorValue = error instanceof Error ? error.message : String(error);
        this.output.appendLine(`ssh_proxy stop-route failed: ${this.lastErrorValue}`);
      }
    } else if (routeId) {
      this.output.appendLine(`ssh_proxy route ${routeId} is shared or not owned by this window; detaching without stop-route`);
    }

    if (clearIntent) {
      await this.stopSessionDaemon();
      this.currentProxy = undefined;
      this.currentSshHostValue = undefined;
      this.childRouteId = undefined;
      this.currentCliKey = undefined;
      this.currentCli = undefined;
      this.currentControl = undefined;
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

  private async ensureLocalService(cli: SshProxyCli, config: RemoteProxyConfig): Promise<SshProxyControlConnection | undefined> {
    const reusedSession = await this.reuseSessionDaemonIfHealthy(cli);
    if (reusedSession) {
      return reusedSession;
    }

    if (config.sshProxyBrokerMode === 'session-only') {
      return this.startSessionDaemon(cli, 'broker mode is session-only for this window');
    }

    if (config.sshProxyBrokerMode === 'disabled') {
      this.output.appendLine('ssh_proxy broker mode is disabled; using the CLI default control endpoint');
      return undefined;
    }

    if (!config.sshProxyPreferPersistentService && config.sshProxyBrokerMode !== 'persistent') {
      return this.startSessionDaemon(cli, 'persistent service preference is disabled for this window');
    }

    const initialStatus = await this.readLocalServiceStatus(cli);
    this.setSnapshot({ serviceStatus: initialStatus });
    if (isSshProxyOk(initialStatus) === true) {
      this.output.appendLine('ssh_proxy local service is healthy; reusing default control endpoint');
      return controlFromServiceStatus(initialStatus);
    }

    if (!config.sshProxyAutoInstallLocalService) {
      return this.startSessionDaemon(cli, `local service is not healthy and auto-install is disabled: ${prettyJson(initialStatus)}`);
    }

    if (this.recovery.isBlocked('persistent-service-install')) {
      return this.startSessionDaemon(
        cli,
        `local service install is blocked for this session: ${this.recovery.reason('persistent-service-install') ?? 'previous permission failure'}; status=${prettyJson(initialStatus)}`,
      );
    }

    this.output.appendLine('ssh_proxy local service is not healthy; attempting resolver-driven ensure');
    try {
      const ensured = await cli.serviceEnsureJson('auto', { elevate: false });
      this.setSnapshot({ serviceStatus: ensured });
      if (isSshProxyOk(ensured) === true) {
        this.output.appendLine('ssh_proxy local service is healthy after ensure; reusing selected control endpoint');
        return controlFromServiceStatus(ensured);
      }
      const nextAction = asString(asRecord(ensured)?.next_action);
      if (nextAction === 'session_daemon' || nextAction === 'install_system_elevated') {
        return this.startSessionDaemon(cli, `service ensure selected ${nextAction}: ${prettyJson(ensured)}`);
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (isPermissionDeniedMessage(message)) {
        this.recovery.recordServiceFailure(message);
      }
      return this.startSessionDaemon(cli, `auto service ensure failed: ${message}`);
    }
    return this.startSessionDaemon(cli, 'service ensure did not produce a reusable control endpoint');
  }

  private async reuseSessionDaemonIfHealthy(cli: SshProxyCli): Promise<SshProxyControlConnection | undefined> {
    const daemon = this.sessionDaemon;
    if (!daemon) {
      return undefined;
    }
    if (daemon.child.exitCode !== null || daemon.child.signalCode !== null) {
      await this.stopSessionDaemon();
      return undefined;
    }
    try {
      const status = await cli.nodeControlStatusJson(daemon);
      const ok = isSshProxyOk(status) === true;
      if (!ok) {
        await this.stopSessionDaemon();
        return undefined;
      }
      this.currentControl = { endpoint: daemon.endpoint, token: daemon.token };
      this.setSnapshot({
        serviceStatus: {
          ok: true,
          mode: 'session-daemon',
          endpoint: daemon.endpoint,
          transport: daemon.transport,
          status,
        },
      });
      this.output.appendLine(`ssh_proxy session daemon is still healthy; reusing ${daemon.endpoint}`);
      return this.currentControl;
    } catch (error) {
      await this.stopSessionDaemon();
      this.output.appendLine(`ssh_proxy session daemon reuse failed: ${error instanceof Error ? error.message : String(error)}`);
      return undefined;
    }
  }

  private async readLocalServiceStatus(cli: SshProxyCli): Promise<unknown> {
    try {
      return await cli.serviceStatusJson();
    } catch (error) {
      return {
        ok: false,
        error: error instanceof Error ? error.message : String(error),
      };
    }
  }

  private async startSessionDaemon(cli: SshProxyCli, reason: string): Promise<SshProxyControlConnection> {
    await this.stopSessionDaemon();
    const daemon = await startSshProxySessionDaemon(cli, this.output, reason);
    this.sessionDaemon = daemon;
    const status = await cli.nodeControlStatusJson(daemon);
    this.setSnapshot({
      serviceStatus: {
        ok: true,
        mode: 'session-daemon',
        endpoint: daemon.endpoint,
        transport: daemon.transport,
        status,
      },
    });
    return { endpoint: daemon.endpoint, token: daemon.token };
  }

  private async stopSessionDaemon(): Promise<void> {
    const daemon = this.sessionDaemon;
    this.sessionDaemon = undefined;
    if (daemon && this.currentCli) {
      await shutdownSshProxySessionDaemon(this.currentCli, daemon, this.output);
    } else if (daemon) {
      daemon.child.kill();
    }
    if (this.currentControl?.endpoint === daemon?.endpoint) {
      this.currentControl = undefined;
    }
  }

  private async stopSessionDaemonIfUnhealthy(): Promise<void> {
    const daemon = this.sessionDaemon;
    if (!daemon || !this.currentCli) {
      return;
    }
    if (daemon.child.exitCode !== null || daemon.child.signalCode !== null) {
      await this.stopSessionDaemon();
      return;
    }
    try {
      const status = await this.currentCli.nodeControlStatusJson(daemon);
      if (isSshProxyOk(status) !== true) {
        await this.stopSessionDaemon();
      }
    } catch {
      await this.stopSessionDaemon();
    }
  }

  private async refreshRouteStatus(cli: SshProxyCli, routeId: string, control: SshProxyControlConnection | undefined): Promise<void> {
    try {
      const routes = await cli.routesJson(control);
      const currentState = this.snapshot.routeState;
      if (currentState) {
        this.setSnapshot({
          routeState: refreshSshProxyRouteState(currentState, routes),
          lastRefreshAt: Date.now(),
        });
      }
      this.output.appendLine(`ssh_proxy routes status captured for ${routeId}`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      this.output.appendLine(`ssh_proxy routes status capture failed for ${routeId}: ${message}`);
    }
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

  private async waitForRouteReadiness(cli: SshProxyCli, routeId: string, control: SshProxyControlConnection | undefined): Promise<void> {
    const deadline = Date.now() + 12_000;
    let lastState = 'unknown';
    let lastError: string | undefined;
    while (Date.now() <= deadline) {
      await this.refreshRouteStatus(cli, routeId, control);
      const liveRoute = this.snapshot.routeState?.liveRoute;
      const state = routeLiveState(liveRoute);
      lastState = state ?? lastState;
      lastError = asString(asRecord(liveRoute)?.last_error)
        ?? asString(asRecord(asRecord(liveRoute)?.stats)?.last_error)
        ?? lastError;
      if (state === 'error' || state === 'failed') {
        throw new Error(`ssh_proxy route ${routeId} failed while starting: ${lastError ?? 'unknown error'}`);
      }
      if (state === 'running' || state === 'ready' || state === 'restarting') {
        return;
      }
      await sleep(300);
    }
    this.output.appendLine(`ssh_proxy route ${routeId} readiness still ${lastState}; continuing to remote port verification`);
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

function controlFromServiceStatus(status: unknown): SshProxyControlConnection | undefined {
  const record = asRecord(status);
  const selected = asRecord(record?.selected_control);
  const selectedEndpoint = asString(selected?.endpoint)
    ?? asString(asRecord(selected?.selected)?.endpoint)
    ?? asString(asRecord(selected?.control)?.endpoint);
  if (selectedEndpoint) {
    return {
      endpoint: selectedEndpoint,
      token: asString(selected?.token),
    };
  }

  const control = asRecord(record?.control);
  const endpoint = asString(record?.endpoint)
    ?? asString(control?.endpoint)
    ?? asString(asRecord(record?.service)?.endpoint);
  if (endpoint) {
    return {
      endpoint,
      token: asString(record?.token) ?? asString(control?.token),
    };
  }

  return undefined;
}

function routeLiveState(liveRoute: unknown): string | undefined {
  const record = asRecord(liveRoute);
  const stats = asRecord(record?.stats);
  const runtime = asRecord(record?.runtime);
  const health = asRecord(record?.health);
  return asString(record?.state)
    ?? asString(stats?.state)
    ?? asString(runtime?.state)
    ?? asString(health?.state);
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
