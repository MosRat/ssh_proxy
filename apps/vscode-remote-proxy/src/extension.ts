import * as vscode from 'vscode';
import { detectActiveSshRemoteCommand } from './activeSshRemote';
import { applyHostProfile, clearSshHost, readConfig, readHostProfile, setManualProxyUrl, setSshHost } from './config';
import {
  appliedProxyFromRemoteStatus,
  buildRemotePortCandidates,
  remoteStatusMatchesCurrentProxy,
} from './portPersistence';
import { detectLocalProxy, findProbeCandidates, makeRemoteProxyUrl } from './proxyDetection';
import { createOwnerId, ProxyLeaseManager, ProxyLeaseState } from './proxyLease';
import { recordHealthCheckFailure, shouldRetryForwardAttempt } from './reliabilityPolicy';
import { getRemoteContext } from './remoteContext';
import { RemoteSetup } from './remoteSetup';
import { shouldReleaseOwnedLease } from './routeOwnership';
import { readSshHostEntries, SshHostEntry } from './sshConfig';
import { OpenSshReverseBackend } from './openSshReverseBackend';
import { ForwardingBackend } from './forwardingBackend';
import { findAvailableSshProxyCli } from './sshProxyCli';
import {
  describeSshProxyDiscovery,
  resolveSshProxyExecutableCandidates,
  sshProxyUnavailableCandidatesMessage,
} from './sshProxyDiscovery';
import { SshProxyKernelBackend } from './sshProxyKernelBackend';
import { SshProxyKernelStatusSnapshot } from './sshProxyKernelStatus';
import {
  buildRemoteProxyStatusLines,
  describeSshProxyDaemonHealth,
  describeSshProxyRouteHealth,
} from './statusDisplay';
import { AppliedProxy, ForwardingBackendKind, LocalProxy, RemoteProxyConfig } from './types';
import { detectSshHostFromVsCodeStorage } from './vscodeStorage';

let controller: RemoteProxyController | undefined;

interface ProxyQuickPickItem extends vscode.QuickPickItem {
  readonly candidate?: LocalProxy;
}

interface SshHostQuickPickItem extends vscode.QuickPickItem {
  readonly entry?: SshHostEntry;
  readonly manual?: boolean;
}

interface ActionQuickPickItem extends vscode.QuickPickItem {
  readonly run: () => Thenable<unknown> | Promise<unknown> | unknown;
}

interface StatusSnapshot {
  readonly lines: string[];
  readonly detectedHost: string | undefined;
  readonly detectedSource: string | undefined;
  readonly detectedConfidence: string;
  readonly proxy: AppliedProxy | undefined;
  readonly lease: ProxyLeaseState | undefined;
  readonly leaseMode: LeaseMode;
  readonly kernelStatus: SshProxyKernelStatusSnapshot | undefined;
  readonly daemonHealth: string;
  readonly routeHealth: string;
}

type LeaseMode = 'none' | 'owner' | 'shared';

export function activate(context: vscode.ExtensionContext): void {
  const output = vscode.window.createOutputChannel('Remote Proxy');
  controller = new RemoteProxyController(output, context);
  context.subscriptions.push(output, controller);

  context.subscriptions.push(
    vscode.commands.registerCommand('remoteProxy.start', () => controller?.start()),
    vscode.commands.registerCommand('remoteProxy.stop', () => controller?.stop()),
    vscode.commands.registerCommand('remoteProxy.restart', () => controller?.restart()),
    vscode.commands.registerCommand('remoteProxy.applySettings', () => controller?.applySettingsOnly()),
    vscode.commands.registerCommand('remoteProxy.cleanupRemote', () => controller?.cleanupRemote()),
    vscode.commands.registerCommand('remoteProxy.pickLocalProxy', () => controller?.pickLocalProxy()),
    vscode.commands.registerCommand('remoteProxy.pickSshHost', () => controller?.pickSshHost()),
    vscode.commands.registerCommand('remoteProxy.clearSshHost', () => controller?.clearSshHost()),
    vscode.commands.registerCommand('remoteProxy.openMenu', () => controller?.openMenu()),
    vscode.commands.registerCommand('remoteProxy.diagnose', () => controller?.diagnose()),
    vscode.commands.registerCommand('remoteProxy.showOutput', () => controller?.showOutput()),
    vscode.commands.registerCommand('remoteProxy.openSettings', () => controller?.openSettings()),
    vscode.commands.registerCommand('remoteProxy.showStatus', () => controller?.showStatus()),
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration('remoteProxy')) {
        controller?.onConfigChanged();
      }
    })
  );

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
  private readonly leaseManager: ProxyLeaseManager;
  private readonly statusBar: vscode.StatusBarItem;
  private readonly monitorTimer: NodeJS.Timeout;
  private readonly ownerId = createOwnerId();
  private applying = false;
  private leaseModeValue: LeaseMode = 'none';
  private leaseTargetKey: string | undefined;
  private lastResolvedTargetKey: string | undefined;
  private lastHealthCheckAt = 0;
  private lastLeaseHeartbeatAt = 0;
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
    this.leaseManager = new ProxyLeaseManager(output, this.ownerId);
    this.statusBar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 30);
    this.statusBar.command = 'remoteProxy.openMenu';
    this.statusBar.tooltip = this.buildStatusTooltip();
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

  private getBackendName(): ForwardingBackendKind {
    return this.forwarder === this.sshProxyBackend ? 'ssh_proxy' : 'openssh';
  }

  public async maybeAutoStart(): Promise<void> {
    let config = readConfig();
    const remote = getRemoteContext(config.sshHostOverride);

    if (!config.enabled || !config.autoStart) {
      this.output.appendLine('Remote Proxy is disabled or autoStart is off.');
      return;
    }

    if (remote.kind === 'none') {
      this.output.appendLine('Not in a remote window; auto-start skipped.');
      return;
    }

    if (remote.kind !== 'ssh') {
      this.output.appendLine(`Remote kind "${remote.name ?? remote.kind}" detected; only SSH is currently auto-started.`);
      this.updateStatusBar(`$(circle-slash) Proxy ${remote.kind}`);
      return;
    }

    await this.start({ interactive: false });
  }

  public async start(options: { interactive: boolean } = { interactive: true }): Promise<void> {
    if (this.applying) {
      return;
    }

    let config = readConfig();
    if (!config.enabled) {
      vscode.window.showInformationMessage('Remote Proxy is disabled.');
      return;
    }

    const remote = getRemoteContext(config.sshHostOverride);
    if (remote.kind !== 'ssh' && !config.sshHostOverride.trim()) {
      vscode.window.showWarningMessage('Remote Proxy currently supports automatic forwarding for Remote SSH windows.');
      this.output.appendLine(`Cannot start: remote=${JSON.stringify(remote)}`);
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
      const targetKey = this.leaseManager.getStableTargetKey(sshHost, this.lastResolvedTargetKey);
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

      await this.ensureForwardingBackend(config);
      const startLock = config.singletonReuseEnabled
        ? await this.leaseManager.acquireStartLock(
          targetKey,
          Math.max(1, config.singletonStartLockTimeoutSeconds) * 1000,
          Math.max(config.singletonLeaseTtlSeconds, config.sshConnectTimeout + 10) * 1000,
        )
        : undefined;
      let reused: AppliedProxy | undefined;
      let applied: AppliedProxy;
      try {
        reused = await this.tryReuseLease(config, sshHost, targetKey);
        applied = reused ?? await this.startForwardOnAvailablePort(config, sshHost, targetKey, local);
        await this.setup.applyAll(config, sshHost, applied);
      } finally {
        await startLock?.release();
      }
      this.restartFailures = 0;
      this.healthFailures = 0;
      this.nextRestartAt = 0;
      this.lastHealthCheckAt = 0;
      this.lastLeaseHeartbeatAt = 0;
      this.rememberPreferredPort(applied);
      vscode.window.showInformationMessage(`${reused ? 'Remote Proxy reused' : 'Remote Proxy active'}: ${applied.remoteUrl}`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      this.output.appendLine(`Start failed: ${message}`);
      if (this.leaseModeValue === 'owner') {
        await this.releaseOwnedLease();
        this.leaseModeValue = 'none';
        this.leaseTargetKey = undefined;
        this.forwarder.stop();
      }
      this.forwarder.fail(message);
      vscode.window.showErrorMessage(`Remote Proxy failed: ${message}`);
    } finally {
      this.applying = false;
      this.updateStatusBar();
    }
  }

  public async stop(): Promise<void> {
    this.rememberPreferredPort(this.forwarder.appliedProxy);
    await this.releaseOwnedLease();
    this.leaseModeValue = 'none';
    this.leaseTargetKey = undefined;
    this.forwarder.stop();
    vscode.window.showInformationMessage('Remote Proxy stopped.');
    this.updateStatusBar();
  }

  public async restart(): Promise<void> {
    this.rememberPreferredPort(this.forwarder.appliedProxy);
    await this.releaseOwnedLease();
    this.leaseModeValue = 'none';
    this.leaseTargetKey = undefined;
    this.forwarder.stop();
    await this.start();
  }

  public async cleanupRemote(): Promise<void> {
    const config = readConfig();
    const remote = getRemoteContext(config.sshHostOverride);

    if (remote.kind !== 'ssh' && !config.sshHostOverride.trim()) {
      vscode.window.showWarningMessage('Remote Proxy cleanup requires a Remote SSH window or remoteProxy.ssh.host.');
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
      await this.releaseOwnedLease();
      this.leaseModeValue = 'none';
      this.leaseTargetKey = undefined;
      this.forwarder.stop();
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

    if (remote.kind !== 'ssh' && !config.sshHostOverride.trim()) {
      vscode.window.showWarningMessage('Remote Proxy settings currently require a Remote SSH window or remoteProxy.ssh.host.');
      return;
    }

    const sshHost = await this.resolveSshHost(config, true);
    if (!sshHost) {
      vscode.window.showWarningMessage('Remote Proxy settings need an SSH host alias. Run "Remote Proxy: Pick SSH Host" or set remoteProxy.ssh.host.');
      return;
    }

    const targetKey = this.leaseManager.getStableTargetKey(sshHost, this.lastResolvedTargetKey);
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
    const manualItem: ProxyQuickPickItem = {
      label: 'Enter proxy URL...',
      description: 'http://127.0.0.1:<port> or socks5://127.0.0.1:<port>'
    };
    const picked = await vscode.window.showQuickPick(
      [
        ...candidates.map((candidate) => ({
          label: candidate.url,
          description: candidate.source,
          candidate
        } satisfies ProxyQuickPickItem)),
        manualItem
      ],
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
    const manualItem: SshHostQuickPickItem = {
      label: 'Enter SSH host...',
      description: 'Use the same host alias you use with ssh or Remote SSH',
      manual: true
    };
    const items: SshHostQuickPickItem[] = [
      ...entries.map((entry) => ({
        label: entry.alias,
        description: entry.source,
        picked: entry.alias === current,
        entry
      } satisfies SshHostQuickPickItem)),
      manualItem
    ];
    const picked = await vscode.window.showQuickPick(
      items,
      {
        title: 'Pick SSH host for Remote Proxy',
        placeHolder: current ? `Current: ${current}` : 'Select a Host from ~/.ssh/config or enter one manually'
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
    this.output.appendLine('Remote Proxy diagnostics');
    this.output.appendLine(snapshot.lines.join('\n'));

    const config = readConfig();
    if (this.forwarder.status === 'running' && snapshot.proxy && this.forwarder.currentSshHost) {
      try {
        await this.setup.verifyForward(config, this.forwarder.currentSshHost, snapshot.proxy);
        this.output.appendLine(`diagnose: remote forwarded port is reachable at ${snapshot.proxy.remoteUrl}`);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        this.output.appendLine(`diagnose: remote forwarded port check failed: ${message}`);
      }
    } else {
      this.output.appendLine('diagnose: forwarder is not running, so remote port verification was skipped.');
    }

    this.output.show(true);
  }

  public async openMenu(): Promise<void> {
    const snapshot = await this.collectStatusSnapshot();
    const items: ActionQuickPickItem[] = [
      {
        label: this.forwarder.status === 'running' ? '$(debug-restart) Restart' : '$(play) Start',
        description: this.forwarder.status === 'running' ? 'Rebuild the SSH reverse tunnel' : 'Start proxy forwarding',
        run: () => this.forwarder.status === 'running' ? this.restart() : this.start()
      },
      {
        label: '$(debug-stop) Stop',
        description: this.forwarder.status === 'running' ? 'Stop the current SSH reverse tunnel' : 'Forwarder is not running',
        run: () => this.stop()
      },
      {
        label: '$(pulse) Diagnose',
        description: 'Print status and verify the remote forwarded port',
        run: () => this.diagnose()
      },
      {
        label: '$(gear) Apply Remote Settings',
        description: snapshot.proxy ? `Write ${snapshot.proxy.remoteUrl} to remote VS Code, terminal, and Git settings` : 'Apply settings from the configured remote port',
        run: () => this.applySettingsOnly()
      },
      {
        label: '$(trash) Clean Remote Settings',
        description: 'Remove managed remote proxy settings, terminal env, server-env block, status file, and Git proxy',
        run: () => this.cleanupRemote()
      },
      {
        label: '$(plug) Pick Local Proxy',
        description: 'Select or enter the local proxy URL',
        run: () => this.pickLocalProxy()
      },
      {
        label: '$(server) Pick SSH Host',
        description: 'Set an explicit SSH host override',
        run: () => this.pickSshHost()
      },
      {
        label: '$(close) Clear SSH Host Override',
        description: 'Return to automatic Remote SSH host detection',
        run: () => this.clearSshHost()
      },
      {
        label: '$(settings-gear) Open Settings',
        description: 'Open Remote Proxy settings',
        run: () => this.openSettings()
      },
      {
        label: '$(output) Show Output',
        description: 'Open the Remote Proxy output channel',
        run: () => this.showOutput()
      }
    ];

    const picked = await vscode.window.showQuickPick(items, {
      title: 'Remote Proxy',
      placeHolder: this.describeMenuPlaceholder(snapshot)
    });
    await picked?.run();
  }

  private async collectStatusSnapshot(): Promise<StatusSnapshot> {
    const config = readConfig();
    const remote = getRemoteContext(config.sshHostOverride);
    const proxy = this.forwarder.appliedProxy;
    const lease = await this.readCurrentLease();
    const activeCommandHost = remote.sshHost ? undefined : await detectActiveSshRemoteCommand();
    const storageHost = remote.sshHost || activeCommandHost ? undefined : await detectSshHostFromVsCodeStorage(this.context, { includeGlobalStorage: true });
    const detectedHost = remote.sshHost ?? activeCommandHost?.host ?? storageHost?.host;
    const detectedSource = remote.sshHostSource ?? activeCommandHost?.source ?? storageHost?.source;
    const detectedConfidence = remote.sshHost ? 'high' : activeCommandHost?.confidence ?? storageHost?.confidence ?? 'none';
    const backend = this.getBackendName();
    const kernelStatus = this.getKernelStatus();
    const daemonHealth = describeSshProxyDaemonHealth(backend, kernelStatus);
    const routeHealth = describeSshProxyRouteHealth(backend, kernelStatus);
    const lines = buildRemoteProxyStatusLines({
      status: this.getEffectiveStatus(),
      backend,
      remoteName: remote.name ?? 'none',
      remoteAuthority: remote.authority ?? 'no authority',
      detectedHost,
      detectedSource,
      detectedConfidence,
      forwardSshHost: this.forwarder.currentSshHost,
      leaseMode: this.leaseModeValue,
      leaseDescription: this.leaseManager.describe(lease),
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
      leaseMode: this.leaseModeValue,
      kernelStatus,
      daemonHealth,
      routeHealth
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
    void this.releaseOwnedLease();
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
    const lease = config.singletonReuseEnabled ? await this.leaseManager.read(targetKey) : undefined;
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
          await this.setup.verifyForward(config, sshHost, active);
        }
        this.leaseModeValue = 'owner';
        this.leaseTargetKey = targetKey;
        await this.leaseManager.write(targetKey, sshHost, active);
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

    const lease = await this.leaseManager.read(targetKey);
    const ttlMs = config.singletonLeaseTtlSeconds * 1000;
    if (!lease) {
      return undefined;
    }
    const fresh = this.leaseManager.isFresh(lease, ttlMs);
    const ownerAlive = this.leaseManager.isOwnerProcessAlive(lease);
    if (!fresh) {
      this.output.appendLine(`Shared proxy lease is stale: ${this.leaseManager.describe(lease)} owner_alive=${ownerAlive}`);
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
      this.leaseModeValue = this.leaseManager.isOwnedByThisInstance(lease) ? 'owner' : 'shared';
      this.leaseTargetKey = targetKey;
      this.forwarder.adoptShared(sshHost, lease.proxy);
      this.output.appendLine(`Reusing shared proxy lease: ${this.leaseManager.describe(lease)}`);
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
      this.leaseModeValue = 'shared';
      this.leaseTargetKey = targetKey;
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
    await this.releaseOwnedLease();
    this.leaseModeValue = 'none';
    this.leaseTargetKey = undefined;
    this.forwarder.stop();
    await this.start({ interactive: false });
  }

  private async monitorHealthAndLease(config: RemoteProxyConfig): Promise<void> {
    if (!config.healthCheckEnabled || !this.leaseTargetKey || !this.forwarder.appliedProxy || !this.forwarder.currentSshHost) {
      return;
    }

    const now = Date.now();
    const heartbeatIntervalMs = Math.min(
      Math.max(1000, config.singletonLeaseTtlSeconds * 500),
      Math.max(1000, config.healthCheckIntervalSeconds * 1000)
    );
    if (this.leaseModeValue === 'owner' && now - this.lastLeaseHeartbeatAt >= heartbeatIntervalMs) {
      await this.leaseManager.heartbeat(this.leaseTargetKey);
      this.lastLeaseHeartbeatAt = now;
    }

    const healthIntervalMs = Math.max(1000, config.healthCheckIntervalSeconds * 1000);
    if (now - this.lastHealthCheckAt < healthIntervalMs) {
      return;
    }
    this.lastHealthCheckAt = now;

    try {
      await this.setup.verifyForward(config, this.forwarder.currentSshHost, this.forwarder.appliedProxy);
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
      await this.releaseOwnedLease();
      this.leaseModeValue = 'none';
      this.leaseTargetKey = undefined;
      this.healthFailures = 0;
      if (!this.canRestartNow(config)) {
        return;
      }
      await this.start({ interactive: false });
    }
  }

  private canRestartNow(config: RemoteProxyConfig): boolean {
    const now = Date.now();
    if (this.nextRestartAt > now) {
      const seconds = Math.ceil((this.nextRestartAt - now) / 1000);
      this.output.appendLine(`Restart is in backoff for ${seconds}s.`);
      return false;
    }

    this.restartFailures += 1;
    const delaySeconds = Math.min(config.restartBackoffMaxSeconds, 5 * 2 ** Math.max(0, this.restartFailures - 1));
    this.nextRestartAt = now + delaySeconds * 1000;
    return true;
  }

  private async releaseOwnedLease(): Promise<void> {
    if (shouldReleaseOwnedLease(this.leaseModeValue, this.leaseTargetKey) && this.leaseTargetKey) {
      await this.leaseManager.release(this.leaseTargetKey);
    }
  }

  private rememberPreferredPort(proxy: AppliedProxy | undefined): void {
    if (!proxy) {
      return;
    }
    this.lastPreferredRemotePort = proxy.remotePort;
    this.lastPreferredRemoteBindHost = proxy.remoteBindHost;
  }

  private async readCurrentLease(): Promise<ProxyLeaseState | undefined> {
    return this.leaseTargetKey ? this.leaseManager.read(this.leaseTargetKey) : undefined;
  }

  private getEffectiveStatus(): string {
    if (this.leaseModeValue === 'shared') {
      return 'running(shared)';
    }
    return this.forwarder.status;
  }

  private getKernelStatus(): SshProxyKernelStatusSnapshot | undefined {
    return this.forwarder === this.sshProxyBackend ? this.sshProxyBackend.kernelStatus : undefined;
  }

  private updateStatusBar(text?: string): void {
    if (text) {
      this.statusBar.text = text;
      return;
    }

    switch (this.forwarder.status) {
      case 'running':
        this.statusBar.text = `$(radio-tower) Proxy ${this.forwarder.currentSshHost ?? ''}${this.forwarder.appliedProxy ? `:${this.forwarder.appliedProxy.remotePort}` : ''}`.trim();
        this.statusBar.backgroundColor = undefined;
        break;
      case 'starting':
        this.statusBar.text = '$(sync~spin) Proxy';
        this.statusBar.backgroundColor = undefined;
        break;
      case 'failed':
        this.statusBar.text = '$(warning) Proxy';
        this.statusBar.backgroundColor = new vscode.ThemeColor('statusBarItem.warningBackground');
        break;
      default:
        this.statusBar.text = '$(circle-large-outline) Proxy';
        this.statusBar.backgroundColor = undefined;
        break;
    }
    this.statusBar.tooltip = this.buildStatusTooltip();
  }

  private buildStatusTooltip(): vscode.MarkdownString {
    const proxy = this.forwarder.appliedProxy;
    const backend = this.getBackendName();
    const kernelStatus = this.getKernelStatus();
    const markdown = new vscode.MarkdownString(undefined, true);
    markdown.isTrusted = true;
    markdown.appendMarkdown('**Remote Proxy**\n\n');
    markdown.appendMarkdown(`Status: \`${this.getEffectiveStatus()}\`\n\n`);
    markdown.appendMarkdown(`Backend: \`${backend}\`\n\n`);
    markdown.appendMarkdown(`Lease: \`${this.leaseModeValue}\`\n\n`);
    markdown.appendMarkdown(`SSH host: \`${this.forwarder.currentSshHost ?? 'not active'}\`\n\n`);
    markdown.appendMarkdown(`Remote proxy: \`${proxy?.remoteUrl ?? 'not active'}\`\n\n`);
    markdown.appendMarkdown(`Route: \`${proxy?.routeId ?? 'not active'}\`\n\n`);
    markdown.appendMarkdown(`Transport: \`${proxy?.selectedTransport ?? 'not active'}\`\n\n`);
    markdown.appendMarkdown(`Fallback: \`${proxy?.fallbackReason ?? 'none'}\`\n\n`);
    markdown.appendMarkdown(`Daemon health: \`${sanitizeMarkdownValue(describeSshProxyDaemonHealth(backend, kernelStatus))}\`\n\n`);
    markdown.appendMarkdown(`Route health: \`${sanitizeMarkdownValue(describeSshProxyRouteHealth(backend, kernelStatus))}\`\n\n`);
    markdown.appendMarkdown(`Local proxy: \`${proxy?.local.url ?? 'not active'}\`\n\n`);
    if (this.forwarder.lastError) {
      markdown.appendMarkdown(`Last error: \`${sanitizeMarkdownValue(this.forwarder.lastError)}\`\n\n`);
    }
    markdown.appendMarkdown('[Open Menu](command:remoteProxy.openMenu) | [Diagnose](command:remoteProxy.diagnose) | [Settings](command:remoteProxy.openSettings)');
    return markdown;
  }

  private describeMenuPlaceholder(snapshot: StatusSnapshot): string {
    const proxy = snapshot.proxy?.remoteUrl ?? 'not active';
    const host = snapshot.detectedHost ?? 'host unresolved';
    const transport = snapshot.proxy?.selectedTransport ?? 'transport unknown';
    return `${this.forwarder.status} | ${host} | ${transport} | ${proxy}`;
  }
}

function sanitizeMarkdownValue(value: string): string {
  return value.replace(/`/g, "'");
}
