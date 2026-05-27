import * as vscode from 'vscode';
import { detectActiveSshRemoteCommand } from './activeSshRemote';
import { registerRemoteProxyCommands } from './commandRegistry';
import { applyHostProfile, clearSshHost, readConfig, readHostProfile, setManualProxyUrl, setSshHost } from './config';
import {
  remoteForwardFailureLine,
  remoteForwardReachableLine,
  REMOTE_PROXY_DIAGNOSTICS_HEADER,
  REMOTE_PROXY_DIAGNOSTICS_SKIP_LINE,
  shouldVerifyRemoteForward,
} from './diagnosticsPresenter';
import {
  appliedProxyFromRemoteStatus,
  buildRemotePortCandidates,
  remoteStatusMatchesCurrentProxy,
} from './portPersistence';
import { detectLocalProxy, findProbeCandidates, makeRemoteProxyUrl } from './proxyDetection';
import { buildLocalProxyPickItems, buildSshHostPickItems, sshHostPickPlaceholder } from './quickPickItems';
import { ProxyLeaseState } from './proxyLease';
import { recordHealthCheckFailure, shouldRetryForwardAttempt } from './reliabilityPolicy';
import { getRemoteContext } from './remoteContext';
import { buildRemoteProxyMenuItems } from './remoteProxyMenu';
import { RemoteSetup } from './remoteSetup';
import { readSshHostEntries } from './sshConfig';
import { OpenSshReverseBackend } from './openSshReverseBackend';
import { ForwardingBackend } from './forwardingBackend';
import {
  decideRestartBackoff,
  healthCheckIntervalMs,
  leaseHeartbeatIntervalMs,
  shouldRunTimedCheck,
} from './healthMonitor';
import { LeaseCoordinator, LeaseMode } from './leaseCoordinator';
import { findAvailableSshProxyCli } from './sshProxyCli';
import {
  describeSshProxyDiscovery,
  resolveSshProxyExecutableCandidates,
  sshProxyUnavailableCandidatesMessage,
} from './sshProxyDiscovery';
import { SshProxyKernelBackend } from './sshProxyKernelBackend';
import { SshProxyKernelStatusSnapshot } from './sshProxyKernelStatus';
import { buildRemoteProxyStatusLines } from './statusDisplay';
import { describeRemoteProxyMenuPlaceholder, updateRemoteProxyStatusBar } from './statusPresenter';
import {
  checkApplySettingsPreflight,
  checkCleanupPreflight,
  checkStartPreflight,
  planAutoStart,
  startLockTimings,
} from './startupCoordinator';
import { AppliedProxy, ForwardingBackendKind, LocalProxy, RemoteProxyConfig } from './types';
import { detectSshHostFromVsCodeStorage } from './vscodeStorage';

let controller: RemoteProxyController | undefined;

interface StatusSnapshot {
  readonly lines: string[];
  readonly detectedHost: string | undefined;
  readonly detectedSource: string | undefined;
  readonly detectedConfidence: string;
  readonly proxy: AppliedProxy | undefined;
  readonly lease: ProxyLeaseState | undefined;
  readonly leaseMode: LeaseMode;
  readonly kernelStatus: SshProxyKernelStatusSnapshot | undefined;
}

export function activate(context: vscode.ExtensionContext): void {
  const output = vscode.window.createOutputChannel('Remote Proxy');
  controller = new RemoteProxyController(output, context);
  context.subscriptions.push(output, controller);
  registerRemoteProxyCommands(context, () => controller);

  void controller.maybeAutoStart();
}

export function deactivate(): void {
  controller?.dispose();
}

class RemoteProxyController implements vscode.Disposable {
  private readonly openSshBackend: OpenSshReverseBackend;
  private readonly sshProxyBackend: SshProxyKernelBackend;
  private forwarder: ForwardingBackend;
  private readonly setup: RemoteSetup;
  private readonly leaseCoordinator: LeaseCoordinator;
  private readonly statusBar: vscode.StatusBarItem;
  private readonly monitorTimer: NodeJS.Timeout;
  private applying = false;
  private lastResolvedTargetKey: string | undefined;
  private lastHealthCheckAt = 0;
  private restartFailures = 0;
  private healthFailures = 0;
  private nextRestartAt = 0;
  private lastPreferredRemotePort: number | undefined;
  private lastPreferredRemoteBindHost: string | undefined;

  public constructor(private readonly output: vscode.OutputChannel, private readonly context: vscode.ExtensionContext) {
    this.openSshBackend = new OpenSshReverseBackend(output);
    this.sshProxyBackend = new SshProxyKernelBackend(output, context.extensionPath);
    this.forwarder = this.openSshBackend;
    this.setup = new RemoteSetup(output, context.extensionPath);
    this.leaseCoordinator = new LeaseCoordinator(output);
    this.statusBar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 30);
    this.statusBar.command = 'remoteProxy.openMenu';
    this.statusBar.tooltip = undefined;
    this.statusBar.show();
    this.openSshBackend.onDidChange(() => this.updateStatusBar());
    this.sshProxyBackend.onDidChange(() => this.updateStatusBar());
    this.monitorTimer = setInterval(() => {
      void this.monitorHostChange();
    }, 5000);
    this.context.subscriptions.push(vscode.window.onDidChangeWindowState((state) => {
      if (state.focused) {
        void this.monitorHostChange();
      }
    }));
    this.updateStatusBar();
  }

  private async ensureForwardingBackend(config: RemoteProxyConfig): Promise<ForwardingBackend> {
    const desired = await this.selectForwardingBackend(config);
    if (desired !== this.forwarder) {
      this.forwarder.stop();
      this.forwarder = desired;
    }
    return this.forwarder;
  }

  private async selectForwardingBackend(config: RemoteProxyConfig): Promise<ForwardingBackend> {
    if (config.backend === 'openssh') {
      return this.openSshBackend;
    }
    if (config.backend === 'ssh_proxy') {
      return this.sshProxyBackend;
    }

    const resolved = await findAvailableSshProxyCli(
      config.sshProxyExecutable,
      this.output,
      { extensionPath: this.context.extensionPath },
    );
    if (resolved) {
      this.output.appendLine(`Using ${describeSshProxyDiscovery(resolved.discovery)} as Remote Proxy kernel.`);
      return this.sshProxyBackend;
    }
    const unavailable = sshProxyUnavailableCandidatesMessage(resolveSshProxyExecutableCandidates(
      config.sshProxyExecutable,
      { extensionPath: this.context.extensionPath },
    ));
    this.output.appendLine(`${unavailable} Falling back to OpenSSH backend.`);
    return this.openSshBackend;
  }

  private async buildForwardingBackendCandidates(config: RemoteProxyConfig): Promise<ForwardingBackend[]> {
    const preferred = await this.selectForwardingBackend(config);
    if (preferred === this.sshProxyBackend) {
      return [this.sshProxyBackend, this.openSshBackend];
    }
    return [this.openSshBackend];
  }

  private describeForwardingBackend(backend: ForwardingBackend): string {
    return backend === this.sshProxyBackend ? 'ssh_proxy kernel' : 'OpenSSH';
  }

  private getBackendName(): ForwardingBackendKind {
    return this.forwarder === this.sshProxyBackend ? 'ssh_proxy' : 'openssh';
  }

  public async maybeAutoStart(): Promise<void> {
    let config = readConfig();
    const remote = getRemoteContext(config.sshHostOverride);
    const decision = planAutoStart(config, remote);
    if (decision.action === 'skip') {
      this.output.appendLine(decision.outputLine);
      if (decision.statusText) {
        this.updateStatusBar(decision.statusText);
      }
      return;
    }

    await this.start({ interactive: false });
  }

  public async start(options: { interactive: boolean } = { interactive: true }): Promise<void> {
    if (this.applying) {
      return;
    }

    let config = readConfig();
    const remote = getRemoteContext(config.sshHostOverride);
    const preflight = checkStartPreflight(config, remote);
    if (preflight.action === 'disabled') {
      vscode.window.showInformationMessage(preflight.informationMessage);
      return;
    }
    if (preflight.action === 'unsupported-remote') {
      vscode.window.showWarningMessage(preflight.warningMessage);
      this.output.appendLine(preflight.outputLine);
      return;
    }

    const sshHost = await this.resolveSshHost(config, options.interactive);
    if (!sshHost) {
      this.output.appendLine(`Cannot start: remote=${JSON.stringify(remote)}`);
      if (options.interactive) {
        vscode.window.showWarningMessage('Remote Proxy needs an SSH host alias. Run "Remote Proxy: Pick SSH Host" or set remoteProxy.ssh.host.');
      } else {
        this.updateStatusBar('$(question) Proxy host');
        const action = await vscode.window.showInformationMessage(
          'Remote Proxy cannot infer the SSH host for this window yet.',
          'Pick SSH Host'
        );
        if (action === 'Pick SSH Host') {
          const picked = await this.pickSshHost();
          if (picked) {
            await this.start({ interactive: false });
          }
        }
      }
      return;
    }

    this.applying = true;
    this.updateStatusBar('$(sync~spin) Proxy');
    try {
      const targetKey = this.leaseCoordinator.getStableTargetKey(sshHost, this.lastResolvedTargetKey);
      config = applyHostProfile(config, readHostProfile([sshHost, targetKey]));
      const local = await detectLocalProxy(config);
      if (!local) {
        const action = await vscode.window.showWarningMessage(
          'No local proxy was detected.',
          'Pick Local Proxy',
          'Open Settings'
        );
        if (action === 'Pick Local Proxy') {
          await this.pickLocalProxy();
        } else if (action === 'Open Settings') {
          await this.openSettings();
        }
        this.output.appendLine('No local proxy detected.');
        return;
      }

      const backendCandidates = await this.buildForwardingBackendCandidates(config);
      const lockTimings = startLockTimings(config);
      const startLock = config.singletonReuseEnabled
        ? await this.leaseCoordinator.acquireStartLock(
          targetKey,
          lockTimings.timeoutMs,
          lockTimings.staleMs,
        )
        : undefined;
      let reused: AppliedProxy | undefined;
      let applied: AppliedProxy | undefined;
      let lastError: unknown;
      try {
        for (const [index, backend] of backendCandidates.entries()) {
          if (this.forwarder !== backend) {
            this.forwarder.stop();
            this.forwarder = backend;
          }

          const backendConfig = this.configForBackend(config, backend);
          try {
            reused = await this.tryReuseLease(backendConfig, sshHost, targetKey);
            applied = reused ?? await this.startForwardOnAvailablePort(backendConfig, sshHost, targetKey, local);
            await this.setup.applyAll(backendConfig, sshHost, applied);
            break;
          } catch (error) {
            lastError = error;
            const message = error instanceof Error ? error.message : String(error);
            this.output.appendLine(`Forward attempt using ${this.describeForwardingBackend(backend)} failed: ${message}`);
            await this.releaseLeaseAndStopForwarder();
            if (index + 1 < backendCandidates.length) {
              this.output.appendLine(`Falling back to ${this.describeForwardingBackend(backendCandidates[index + 1])} backend.`);
            }
            if (index + 1 >= backendCandidates.length) {
              throw error;
            }
          }
        }
        if (!applied) {
          throw lastError instanceof Error ? lastError : new Error('no forwarding backend could start');
        }
      } finally {
        await startLock?.release();
      }
      this.restartFailures = 0;
      this.healthFailures = 0;
      this.nextRestartAt = 0;
      this.lastHealthCheckAt = 0;
      this.leaseCoordinator.resetHeartbeat();
      const active = applied;
      if (!active) {
        throw new Error('forwarding backend did not produce an applied proxy');
      }
      this.rememberPreferredPort(active);
      vscode.window.showInformationMessage(`${reused ? 'Remote Proxy reused' : 'Remote Proxy active'}: ${active.remoteUrl}`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      this.output.appendLine(`Start failed: ${message}`);
      if (this.leaseCoordinator.mode === 'owner') {
        await this.releaseLeaseAndStopForwarder();
      }
      this.forwarder.fail(message);
      vscode.window.showErrorMessage(`Remote Proxy failed: ${message}`);
    } finally {
      this.applying = false;
      this.updateStatusBar();
    }
  }

  public async stop(): Promise<void> {
    await this.releaseLeaseAndStopForwarder({ rememberPreferredPort: true });
    vscode.window.showInformationMessage('Remote Proxy stopped.');
    this.updateStatusBar();
  }

  public async restart(): Promise<void> {
    await this.releaseLeaseAndStopForwarder({ rememberPreferredPort: true });
    await this.start();
  }

  public async cleanupRemote(): Promise<void> {
    const config = readConfig();
    const remote = getRemoteContext(config.sshHostOverride);
    const preflight = checkCleanupPreflight(config, remote);

    if (preflight.action === 'unsupported-remote') {
      vscode.window.showWarningMessage(preflight.warningMessage);
      return;
    }

    const sshHost = await this.resolveSshHost(config, true);
    if (!sshHost) {
      vscode.window.showWarningMessage('Remote Proxy cleanup needs an SSH host alias.');
      return;
    }

    const confirmation = await vscode.window.showWarningMessage(
      `Clean Remote Proxy settings on ${sshHost}? This removes managed VS Code machine proxy settings, terminal proxy env, server-env-setup block, remote status file, and global Git proxy config.`,
      { modal: true },
      'Clean Remote'
    );
    if (confirmation !== 'Clean Remote') {
      return;
    }

    try {
      await this.releaseLeaseAndStopForwarder();
      await this.setup.cleanupAll(config, sshHost);
      vscode.window.showInformationMessage(`Remote Proxy settings cleaned on ${sshHost}.`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      this.output.appendLine(`Cleanup failed: ${message}`);
      vscode.window.showErrorMessage(`Remote Proxy cleanup failed: ${message}`);
    } finally {
      this.updateStatusBar();
    }
  }

  public async applySettingsOnly(): Promise<void> {
    let config = readConfig();
    const remote = getRemoteContext(config.sshHostOverride);
    const preflight = checkApplySettingsPreflight(config, remote);

    if (preflight.action === 'unsupported-remote') {
      vscode.window.showWarningMessage(preflight.warningMessage);
      return;
    }

    const sshHost = await this.resolveSshHost(config, true);
    if (!sshHost) {
      vscode.window.showWarningMessage('Remote Proxy settings need an SSH host alias. Run "Remote Proxy: Pick SSH Host" or set remoteProxy.ssh.host.');
      return;
    }

    const targetKey = this.leaseCoordinator.getStableTargetKey(sshHost, this.lastResolvedTargetKey);
    config = applyHostProfile(config, readHostProfile([sshHost, targetKey]));
    const applied = this.forwarder.status === 'running' ? this.forwarder.appliedProxy : await this.buildAppliedProxy(config);

    if (!applied) {
      vscode.window.showWarningMessage('No local proxy was detected.');
      return;
    }

    try {
      await this.setup.applyAll(config, sshHost, applied);
      vscode.window.showInformationMessage(`Remote Proxy settings applied: ${applied.remoteUrl}`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      this.output.appendLine(`Apply settings failed: ${message}`);
      vscode.window.showErrorMessage(`Remote Proxy settings failed: ${message}`);
    }
  }

  public async pickLocalProxy(): Promise<void> {
    const config = readConfig();
    const candidates = await findProbeCandidates(config);
    const picked = await vscode.window.showQuickPick(
      buildLocalProxyPickItems(candidates),
      { title: 'Pick local proxy for remote forwarding' }
    );

    if (!picked) {
      return;
    }

    if (picked.candidate) {
      await setManualProxyUrl(picked.candidate.url);
      vscode.window.showInformationMessage(`Remote Proxy local proxy set to ${picked.candidate.url}`);
      return;
    }

    const value = await vscode.window.showInputBox({
      title: 'Local proxy URL',
      prompt: 'Use a local proxy URL such as http://127.0.0.1:<port> or socks5://127.0.0.1:<port>',
      value: config.localProxyUrl || ''
    });
    if (value) {
      await setManualProxyUrl(value);
      vscode.window.showInformationMessage(`Remote Proxy local proxy set to ${value}`);
    }
  }

  public async pickSshHost(): Promise<string | undefined> {
    const config = readConfig();
    const remote = getRemoteContext(config.sshHostOverride);
    const entries = await readSshHostEntries();
    const current = remote.sshHost ?? config.sshHostOverride;
    const items = buildSshHostPickItems(entries, current);
    const picked = await vscode.window.showQuickPick(
      items,
      {
        title: 'Pick SSH host for Remote Proxy',
        placeHolder: sshHostPickPlaceholder(current)
      }
    );

    if (!picked) {
      return undefined;
    }

    let host = picked.entry?.alias;
    if (picked.manual) {
      host = await vscode.window.showInputBox({
        title: 'SSH host alias',
        prompt: 'Use the same value you pass to ssh, for example my-server or user@example.com',
        value: current || ''
      });
    }

    host = host?.trim();
    if (!host) {
      return undefined;
    }

    await setSshHost(host);
    vscode.window.showInformationMessage(`Remote Proxy SSH host set to ${host}`);
    return host;
  }

  public async clearSshHost(): Promise<void> {
    await clearSshHost();
    vscode.window.showInformationMessage('Remote Proxy SSH host override cleared.');
  }

  public async showStatus(): Promise<void> {
    const snapshot = await this.collectStatusSnapshot();
    this.output.appendLine(snapshot.lines.join('\n'));
    this.output.show(true);
  }

  public showOutput(): void {
    this.output.show(true);
  }

  public async openSettings(): Promise<void> {
    await vscode.commands.executeCommand('workbench.action.openSettings', '@ext:MosRat-2333.vscode-remote-proxy');
  }

  public async diagnose(): Promise<void> {
    const snapshot = await this.collectStatusSnapshot();
    this.output.appendLine(REMOTE_PROXY_DIAGNOSTICS_HEADER);
    this.output.appendLine(snapshot.lines.join('\n'));

    const config = readConfig();
    const setupConfig = this.configForBackend(config, this.forwarder);
    const sshHost = this.forwarder.currentSshHost;
    if (sshHost && snapshot.proxy && shouldVerifyRemoteForward(this.forwarder.status, snapshot.proxy, sshHost)) {
      try {
        await this.setup.verifyForward(setupConfig, sshHost, snapshot.proxy);
        this.output.appendLine(remoteForwardReachableLine(snapshot.proxy));
      } catch (error) {
        this.output.appendLine(remoteForwardFailureLine(error));
      }
    } else {
      this.output.appendLine(REMOTE_PROXY_DIAGNOSTICS_SKIP_LINE);
    }

    this.output.show(true);
  }

  public async openMenu(): Promise<void> {
    const snapshot = await this.collectStatusSnapshot();
    const items = buildRemoteProxyMenuItems(this.forwarder.status, snapshot, {
      start: () => this.start(),
      restart: () => this.restart(),
      stop: () => this.stop(),
      diagnose: () => this.diagnose(),
      applySettingsOnly: () => this.applySettingsOnly(),
      cleanupRemote: () => this.cleanupRemote(),
      pickLocalProxy: () => this.pickLocalProxy(),
      pickSshHost: () => this.pickSshHost(),
      clearSshHost: () => this.clearSshHost(),
      openSettings: () => this.openSettings(),
      showOutput: () => this.showOutput(),
    });

    const picked = await vscode.window.showQuickPick(items, {
      title: 'Remote Proxy',
      placeHolder: describeRemoteProxyMenuPlaceholder(snapshot, this.forwarder.status)
    });
    await picked?.run();
  }

  private async collectStatusSnapshot(): Promise<StatusSnapshot> {
    const config = readConfig();
    const remote = getRemoteContext(config.sshHostOverride);
    const proxy = this.forwarder.appliedProxy;
    const lease = await this.leaseCoordinator.readCurrent();
    const activeCommandHost = remote.sshHost ? undefined : await detectActiveSshRemoteCommand();
    const storageHost = remote.sshHost || activeCommandHost ? undefined : await detectSshHostFromVsCodeStorage(this.context, { includeGlobalStorage: true });
    const detectedHost = remote.sshHost ?? activeCommandHost?.host ?? storageHost?.host;
    const detectedSource = remote.sshHostSource ?? activeCommandHost?.source ?? storageHost?.source;
    const detectedConfidence = remote.sshHost ? 'high' : activeCommandHost?.confidence ?? storageHost?.confidence ?? 'none';
    const backend = this.getBackendName();
    const kernelStatus = this.getKernelStatus();
    const lines = buildRemoteProxyStatusLines({
      status: this.getEffectiveStatus(),
      backend,
      remoteName: remote.name ?? 'none',
      remoteAuthority: remote.authority ?? 'no authority',
      detectedHost,
      detectedSource,
      detectedConfidence,
      forwardSshHost: this.forwarder.currentSshHost,
      leaseMode: this.leaseCoordinator.mode,
      leaseDescription: this.leaseCoordinator.describe(lease),
      restartBackoff: this.nextRestartAt > Date.now() ? `${Math.ceil((this.nextRestartAt - Date.now()) / 1000)}s` : 'ready',
      proxy,
      lease,
      kernelStatus,
      lastError: this.forwarder.lastError,
    });
    return {
      lines,
      detectedHost,
      detectedSource,
      detectedConfidence,
      proxy,
      lease,
      leaseMode: this.leaseCoordinator.mode,
      kernelStatus,
    };
  }

  public onConfigChanged(): void {
    const config = readConfig();
    if (!config.enabled) {
      this.forwarder.stop();
      return;
    }
    this.updateStatusBar();
  }

  public dispose(): void {
    clearInterval(this.monitorTimer);
    void this.leaseCoordinator.releaseOwned();
    this.openSshBackend.dispose();
    this.sshProxyBackend.dispose();
    this.statusBar.dispose();
  }

  private async buildAppliedProxy(config: RemoteProxyConfig): Promise<AppliedProxy | undefined> {
    const local = await detectLocalProxy(config);
    if (!local) {
      return undefined;
    }

    return {
      local,
      remoteUrl: makeRemoteProxyUrl(local, config.remoteBindHost, config.remotePort),
      remotePort: config.remotePort,
      remoteBindHost: config.remoteBindHost
    };
  }

  private async resolveSshHost(config: RemoteProxyConfig, interactive: boolean): Promise<string | undefined> {
    const remote = getRemoteContext(config.sshHostOverride);
    if (remote.sshHost) {
      this.lastResolvedTargetKey = remote.sshHost;
      return remote.sshHost;
    }

    const activeCommandHost = await detectActiveSshRemoteCommand();
    if (activeCommandHost) {
      this.output.appendLine(`Resolved SSH host from Remote SSH active command: ${activeCommandHost.host} (${activeCommandHost.source})`);
      this.lastResolvedTargetKey = activeCommandHost.targetKey ?? activeCommandHost.host;
      return activeCommandHost.host;
    }

    const storageHost = await detectSshHostFromVsCodeStorage(this.context, { includeGlobalStorage: config.sshUseStorageFallback });
    if (storageHost?.confidence === 'high') {
      this.output.appendLine(`Resolved SSH host from VS Code storage: ${storageHost.host} (${storageHost.source})`);
      this.lastResolvedTargetKey = storageHost.targetKey ?? storageHost.host;
      return storageHost.host;
    }
    if (storageHost) {
      this.output.appendLine(`Ignored low-confidence SSH host candidate from VS Code storage: ${storageHost.host} (${storageHost.source})`);
    }

    if (!interactive) {
      return undefined;
    }

    const picked = await this.pickSshHost();
    this.lastResolvedTargetKey = picked;
    return picked;
  }

  private async startForwardOnAvailablePort(config: RemoteProxyConfig, sshHost: string, targetKey: string, local: LocalProxy): Promise<AppliedProxy> {
    const lease = config.singletonReuseEnabled ? await this.leaseCoordinator.read(targetKey) : undefined;
    const remoteStatus = await this.setup.readRemoteStatus(config, sshHost);
    const currentProxy = this.forwarder.currentSshHost === sshHost ? this.forwarder.appliedProxy : undefined;
    const ports = buildRemotePortCandidates({
      config,
      local,
      currentProxy,
      preferredPort: this.lastPreferredRemotePort,
      preferredBindHost: this.lastPreferredRemoteBindHost,
      lease,
      remoteStatus,
    });
    if (remoteStatusMatchesCurrentProxy(remoteStatus, config, local)) {
      this.output.appendLine(`Remote status suggests prior Remote Proxy port ${remoteStatus?.bindHost ?? config.remoteBindHost}:${remoteStatus?.port}; trying it before new ports.`);
    }
    let lastError: unknown;

    for (const port of ports) {
      const applied: AppliedProxy = {
        local,
        remoteUrl: makeRemoteProxyUrl(local, config.remoteBindHost, port),
        remotePort: port,
        remoteBindHost: config.remoteBindHost
      };

      if (!config.remoteAutoPickPort && remoteStatus?.port === port) {
        const free = await this.setup.isRemotePortFree(config, sshHost, config.remoteBindHost, port);
        if (!free) {
          const residual = await this.tryReuseRemoteStatusResidual(config, sshHost, targetKey, local, remoteStatus, port);
          if (residual) {
            return residual;
          }
        }
      }

      if (config.remoteAutoPickPort) {
        const free = await this.setup.isRemotePortFree(config, sshHost, config.remoteBindHost, port);
        if (!free) {
          const residual = await this.tryReuseRemoteStatusResidual(config, sshHost, targetKey, local, remoteStatus, port);
          if (residual) {
            return residual;
          }
          this.output.appendLine(`Remote port ${config.remoteBindHost}:${port} is already in use; trying next port.`);
          continue;
        }
      }

      try {
        await this.forwarder.startAndWait(config, sshHost, applied, 900);
        const active = this.forwarder.appliedProxy ?? applied;
        if (config.verifyAfterStart) {
          await this.setup.verifyForwardReady(config, sshHost, active);
        }
        this.leaseCoordinator.markOwner(targetKey);
        await this.leaseCoordinator.write(targetKey, sshHost, active);
        return active;
      } catch (error) {
        lastError = error;
        this.forwarder.stop();
        const message = error instanceof Error ? error.message : String(error);
        this.output.appendLine(`Forward attempt failed on ${config.remoteBindHost}:${port}: ${message}`);
        if (!config.remoteAutoPickPort || !shouldRetryForwardAttempt(error)) {
          throw error;
        }
      }
    }

    throw lastError instanceof Error ? lastError : new Error(`No available remote proxy port in ${ports[0]}-${ports[ports.length - 1]}.`);
  }

  private async tryReuseLease(config: RemoteProxyConfig, sshHost: string, targetKey: string): Promise<AppliedProxy | undefined> {
    if (!config.singletonReuseEnabled) {
      return undefined;
    }

    const lease = await this.leaseCoordinator.read(targetKey);
    const ttlMs = config.singletonLeaseTtlSeconds * 1000;
    if (!lease) {
      return undefined;
    }
    const fresh = this.leaseCoordinator.isFresh(lease, ttlMs);
    const ownerAlive = this.leaseCoordinator.isOwnerProcessAlive(lease);
    if (!fresh) {
      this.output.appendLine(`Shared proxy lease is stale: ${this.leaseCoordinator.describe(lease)} owner_alive=${ownerAlive}`);
    }

    const backendName = this.getBackendName();
    if (backendName === 'ssh_proxy') {
      if (lease.version !== 2 || lease.backend !== 'ssh_proxy') {
        return undefined;
      }
    } else if (lease.version === 2 && lease.backend === 'ssh_proxy') {
      return undefined;
    }

    try {
      await this.setup.verifyForward(config, sshHost, lease.proxy);
      this.leaseCoordinator.markFromLease(targetKey, lease);
      this.forwarder.adoptShared(sshHost, lease.proxy);
      this.output.appendLine(`Reusing shared proxy lease: ${this.leaseCoordinator.describe(lease)}`);
      return lease.proxy;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      const leaseAge = fresh ? 'fresh' : ownerAlive ? 'stale owner-alive' : 'stale owner-dead';
      this.output.appendLine(`Shared proxy lease (${leaseAge}) is not reachable and will not be reused: ${message}`);
      return undefined;
    }
  }

  private async tryReuseRemoteStatusResidual(
    config: RemoteProxyConfig,
    sshHost: string,
    targetKey: string,
    local: LocalProxy,
    remoteStatus: Awaited<ReturnType<RemoteSetup['readRemoteStatus']>>,
    port: number,
  ): Promise<AppliedProxy | undefined> {
    if (!remoteStatus || remoteStatus.port !== port) {
      return undefined;
    }
    const proxy = appliedProxyFromRemoteStatus(remoteStatus, config, local);
    if (!proxy) {
      return undefined;
    }
    try {
      await this.setup.verifyForward(config, sshHost, proxy);
      this.leaseCoordinator.markShared(targetKey);
      this.forwarder.adoptShared(sshHost, proxy);
      this.rememberPreferredPort(proxy);
      this.output.appendLine(`Remote port ${proxy.remoteBindHost}:${proxy.remotePort} matches previous Remote Proxy status; reusing existing listener.`);
      return proxy;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      this.output.appendLine(`Previous Remote Proxy status port ${proxy.remoteBindHost}:${proxy.remotePort} is not reusable: ${message}`);
      return undefined;
    }
  }

  private async monitorHostChange(): Promise<void> {
    if (this.applying || this.forwarder.status !== 'running') {
      return;
    }

    const config = readConfig();
    if (!config.enabled || !config.autoStart) {
      return;
    }

    const remote = getRemoteContext(config.sshHostOverride);
    if (remote.kind !== 'ssh') {
      return;
    }

    if (!config.restartOnHostChange) {
      await this.monitorHealthAndLease(config);
      return;
    }

    const expectedHost = await this.resolveSshHost(config, false);
    const currentHost = this.forwarder.currentSshHost;
    if (!expectedHost || !currentHost || expectedHost === currentHost) {
      await this.monitorHealthAndLease(config);
      return;
    }

    this.output.appendLine(`Detected Remote SSH host change: ${currentHost} -> ${expectedHost}. Restarting proxy.`);
    await this.releaseLeaseAndStopForwarder();
    await this.start({ interactive: false });
  }

  private async monitorHealthAndLease(config: RemoteProxyConfig): Promise<void> {
    if (!config.healthCheckEnabled || !this.leaseCoordinator.targetKey || !this.forwarder.appliedProxy || !this.forwarder.currentSshHost) {
      return;
    }

    const now = Date.now();
    const heartbeatIntervalMs = leaseHeartbeatIntervalMs(
      config.singletonLeaseTtlSeconds,
      config.healthCheckIntervalSeconds,
    );
    if (this.leaseCoordinator.mode === 'owner' && shouldRunTimedCheck(now, this.leaseCoordinator.lastHeartbeatAt, heartbeatIntervalMs)) {
      await this.leaseCoordinator.heartbeatCurrent();
      this.leaseCoordinator.markHeartbeat(now);
    }

    const healthIntervalMs = healthCheckIntervalMs(config.healthCheckIntervalSeconds);
    if (!shouldRunTimedCheck(now, this.lastHealthCheckAt, healthIntervalMs)) {
      return;
    }
    this.lastHealthCheckAt = now;

    try {
      const setupConfig = this.configForBackend(config, this.forwarder);
      await this.setup.verifyForward(setupConfig, this.forwarder.currentSshHost, this.forwarder.appliedProxy);
      if (this.healthFailures > 0) {
        this.output.appendLine(`Proxy health check recovered after ${this.healthFailures} failed attempt(s).`);
      }
      this.healthFailures = 0;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      const decision = recordHealthCheckFailure(this.healthFailures, config.healthCheckFailureThreshold);
      this.healthFailures = decision.failures;
      this.output.appendLine(`Proxy health check failed (${decision.failures}/${decision.threshold}): ${message}`);
      if (!decision.shouldRestart) {
        return;
      }
      this.forwarder.fail(message);
      await this.leaseCoordinator.releaseOwned();
      this.leaseCoordinator.clear();
      this.healthFailures = 0;
      if (!this.canRestartNow(config)) {
        return;
      }
      await this.start({ interactive: false });
    }
  }

  private canRestartNow(config: RemoteProxyConfig): boolean {
    const decision = decideRestartBackoff(
      Date.now(),
      this.nextRestartAt,
      this.restartFailures,
      config.restartBackoffMaxSeconds,
    );
    this.restartFailures = decision.restartFailures;
    this.nextRestartAt = decision.nextRestartAt;
    if (!decision.canRestart) {
      this.output.appendLine(`Restart is in backoff for ${decision.waitSeconds}s.`);
      return false;
    }

    return true;
  }

  private async releaseLeaseAndStopForwarder(options: { rememberPreferredPort?: boolean } = {}): Promise<void> {
    if (options.rememberPreferredPort) {
      this.rememberPreferredPort(this.forwarder.appliedProxy);
    }
    await this.leaseCoordinator.releaseOwned();
    this.leaseCoordinator.clear();
    this.forwarder.stop();
  }

  private rememberPreferredPort(proxy: AppliedProxy | undefined): void {
    if (!proxy) {
      return;
    }
    this.lastPreferredRemotePort = proxy.remotePort;
    this.lastPreferredRemoteBindHost = proxy.remoteBindHost;
  }

  private getEffectiveStatus(): string {
    if (this.leaseCoordinator.mode === 'shared') {
      return 'running(shared)';
    }
    return this.forwarder.status;
  }

  private getKernelStatus(): SshProxyKernelStatusSnapshot | undefined {
    return this.forwarder === this.sshProxyBackend ? this.sshProxyBackend.kernelStatus : undefined;
  }

  private updateStatusBar(text?: string): void {
    updateRemoteProxyStatusBar(this.statusBar, {
      status: this.forwarder.status,
      effectiveStatus: this.getEffectiveStatus(),
      backend: this.getBackendName(),
      leaseMode: this.leaseCoordinator.mode,
      sshHost: this.forwarder.currentSshHost,
      proxy: this.forwarder.appliedProxy,
      kernelStatus: this.getKernelStatus(),
      lastError: this.forwarder.lastError,
    }, text);
  }

  private configForBackend(config: RemoteProxyConfig, backend: ForwardingBackend): RemoteProxyConfig {
    if (backend !== this.openSshBackend || config.sshProxyRemoteSetup === 'openssh') {
      return config;
    }
    return {
      ...config,
      sshProxyRemoteSetup: 'openssh',
    };
  }
}
