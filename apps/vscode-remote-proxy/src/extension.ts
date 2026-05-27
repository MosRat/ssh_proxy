import * as vscode from 'vscode';
import { detectActiveSshRemoteCommand } from './activeSshRemote';
import { registerRemoteProxyCommands } from './commandRegistry';
import { applyHostProfile, clearSshHost, readConfig, readHostProfile, setManualProxyUrl, setSshHost } from './config';
import {
  REMOTE_PROXY_DIAGNOSTICS_HEADER,
  REMOTE_PROXY_DIAGNOSTICS_SKIP_LINE,
} from './diagnosticsPresenter';
import { detectLocalProxy, findProbeCandidates, makeRemoteProxyUrl } from './proxyDetection';
import { buildLocalProxyPickItems, buildSshHostPickItems, sshHostPickPlaceholder } from './quickPickItems';
import { recordHealthCheckFailure } from './reliabilityPolicy';
import { getRemoteContext } from './remoteContext';
import { buildRemoteProxyMenuItems } from './remoteProxyMenu';
import { readSshHostEntries } from './sshConfig';
import { ForwardingBackend } from './forwardingBackend';
import { healthCheckIntervalMs, shouldRunTimedCheck } from './healthMonitor';
import { findAvailableSshProxyCli } from './sshProxyCli';
import {
  describeSshProxyDiscovery,
  resolveSshProxyExecutableCandidates,
  sshProxyUnavailableCandidatesMessage,
} from './sshProxyDiscovery';
import { isSshProxyDaemonInstallCancelledMessage } from './sshProxyCliUtils';
import { isSshProxyDaemonRejectedError, SshProxyKernelBackend } from './sshProxyKernelBackend';
import { SshProxyKernelStatusSnapshot } from './sshProxyKernelStatus';
import { buildRemoteProxyStatusLines } from './statusDisplay';
import { describeRemoteProxyMenuPlaceholder, updateRemoteProxyStatusBar } from './statusPresenter';
import {
  checkApplySettingsPreflight,
  checkCleanupPreflight,
  checkStartPreflight,
  planAutoStart,
} from './startupCoordinator';
import { AppliedProxy, ForwardingBackendKind, RemoteProxyConfig } from './types';
import { detectSshHostFromVsCodeStorage } from './vscodeStorage';

let controller: RemoteProxyController | undefined;

interface StatusSnapshot {
  readonly lines: string[];
  readonly detectedHost: string | undefined;
  readonly detectedSource: string | undefined;
  readonly detectedConfidence: string;
  readonly proxy: AppliedProxy | undefined;
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
  private readonly sshProxyBackend: SshProxyKernelBackend;
  private forwarder: ForwardingBackend;
  private readonly statusBar: vscode.StatusBarItem;
  private readonly monitorTimer: NodeJS.Timeout;
  private applying = false;
  private lastResolvedTargetKey: string | undefined;
  private lastHealthCheckAt = 0;
  private healthFailures = 0;

  public constructor(private readonly output: vscode.OutputChannel, private readonly context: vscode.ExtensionContext) {
    this.sshProxyBackend = new SshProxyKernelBackend(output, context.extensionPath);
    this.forwarder = this.sshProxyBackend;
    this.statusBar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 30);
    this.statusBar.command = 'remoteProxy.openMenu';
    this.statusBar.tooltip = undefined;
    this.statusBar.show();
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
    const resolved = await findAvailableSshProxyCli(
      config.sshProxyExecutable,
      this.output,
      { extensionPath: this.context.extensionPath },
    );
    if (resolved) {
      void config;
      this.output.appendLine(`Using ${describeSshProxyDiscovery(resolved.discovery)} as Remote Proxy daemon client.`);
      return this.sshProxyBackend;
    }
    const unavailable = sshProxyUnavailableCandidatesMessage(resolveSshProxyExecutableCandidates(
      config.sshProxyExecutable,
      { extensionPath: this.context.extensionPath },
    ));
    this.output.appendLine(`${unavailable} ssh_proxy daemon client is required in production mode.`);
    return this.sshProxyBackend;
  }

  private getBackendName(): ForwardingBackendKind {
    return 'ssh_proxy';
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
    let retryAfterDaemonInstall = false;
    try {
      const targetKey = this.lastResolvedTargetKey ?? sshHost;
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

      const backend = await this.ensureForwardingBackend(config);
      const applied: AppliedProxy = {
        local,
        remoteUrl: makeRemoteProxyUrl(local, config.remoteBindHost, config.remotePort),
        remotePort: config.remotePort,
        remoteBindHost: config.remoteBindHost,
        workspaceId: targetKey,
      };
      this.output.appendLine(`Starting daemon-owned proxy session for ${sshHost} (${targetKey}).`);
      await backend.startAndWait(config, sshHost, applied, 900);
      this.healthFailures = 0;
      this.lastHealthCheckAt = 0;
      const active = backend.appliedProxy ?? applied;
      if (!active) {
        throw new Error('forwarding backend did not produce an applied proxy');
      }
      vscode.window.showInformationMessage(`Remote Proxy active: ${active.remoteUrl}`);
    } catch (error) {
      const failure = await this.handleStartFailure(error, options, config);
      if (failure === 'retry') {
        retryAfterDaemonInstall = true;
      } else if (failure !== 'handled') {
        const message = error instanceof Error ? error.message : String(error);
        this.output.appendLine(`Start failed: ${message}`);
        this.forwarder.fail(message);
        vscode.window.showErrorMessage(`Remote Proxy failed: ${message}`);
      }
    } finally {
      this.applying = false;
      this.updateStatusBar();
    }
    if (retryAfterDaemonInstall) {
      await this.start({ interactive: false });
    }
  }

  public async stop(): Promise<void> {
    await this.stopForwarder();
    vscode.window.showInformationMessage('Remote Proxy stopped.');
    this.updateStatusBar();
  }

  public async restart(): Promise<void> {
    await this.stopForwarder();
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
      await this.stopForwarder();
      vscode.window.showInformationMessage(`Remote Proxy cleanup requested through daemon on ${sshHost}.`);
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

    try {
      const targetKey = this.lastResolvedTargetKey ?? sshHost;
      config = applyHostProfile(config, readHostProfile([sshHost, targetKey]));
      const activeProxy = this.forwarder.appliedProxy;
      const local = activeProxy?.local ?? await detectLocalProxy(config);
      if (!local) {
        vscode.window.showWarningMessage('Remote Proxy settings need a detectable local proxy or an active daemon session.');
        return;
      }
      const proxyUrl = activeProxy?.remoteUrl ?? makeRemoteProxyUrl(local, config.remoteBindHost, config.remotePort);
      const resolved = await findAvailableSshProxyCli(
        config.sshProxyExecutable,
        this.output,
        { extensionPath: this.context.extensionPath },
      );
      if (!resolved) {
        throw new Error(sshProxyUnavailableCandidatesMessage(resolveSshProxyExecutableCandidates(
          config.sshProxyExecutable,
          { extensionPath: this.context.extensionPath },
        )));
      }
      const result = await resolved.cli.vscodeApplySettingsJson({
        target: sshHost,
        workspace: targetKey,
        proxyUrl,
      });
      this.output.appendLine(`ssh_proxy vscode apply-settings: ${JSON.stringify(result)}`);
      await this.sshProxyBackend.refreshStatus();
      vscode.window.showInformationMessage(`Remote Proxy settings apply requested through daemon on ${sshHost}.`);
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

  private async handleStartFailure(
    error: unknown,
    options: { interactive: boolean },
    config: RemoteProxyConfig,
  ): Promise<'handled' | 'retry' | 'unhandled'> {
    if (!isSshProxyDaemonRejectedError(error)) {
      return 'unhandled';
    }

    const message = error.userMessage;
    this.output.appendLine(`Start blocked: ${message}`);
    if (error.nextAction) {
      this.output.appendLine(`Daemon repair action: ${error.nextAction}`);
    }
    this.forwarder.fail(message);

    if (!options.interactive) {
      this.updateStatusBar('$(warning) Proxy daemon');
      return 'handled';
    }

    const installAction = 'Install Daemon';
    const diagnoseAction = 'Diagnose';
    const showOutputAction = 'Show Output';
    const actions = error.requiresDaemon || error.requiresElevation
      ? [installAction, diagnoseAction, showOutputAction]
      : [diagnoseAction, showOutputAction];
    const picked = await vscode.window.showErrorMessage(
      `${message}.`,
      ...actions,
    );

    if (picked === installAction) {
      const installed = await this.installDaemonWithElevation(config);
      return installed ? 'retry' : 'handled';
    }
    if (picked === diagnoseAction) {
      await this.diagnose();
      return 'handled';
    }
    if (picked === showOutputAction) {
      this.output.show(true);
      return 'handled';
    }
    return 'handled';
  }

  private async installDaemonWithElevation(config: RemoteProxyConfig): Promise<boolean> {
    try {
      const resolved = await findAvailableSshProxyCli(
        config.sshProxyExecutable,
        this.output,
        { extensionPath: this.context.extensionPath },
      );
      if (!resolved) {
        throw new Error(sshProxyUnavailableCandidatesMessage(resolveSshProxyExecutableCandidates(
          config.sshProxyExecutable,
          { extensionPath: this.context.extensionPath },
        )));
      }
      await vscode.window.withProgress(
        {
          location: vscode.ProgressLocation.Notification,
          title: 'Installing ssh_proxy daemon. Approve the Windows UAC prompt if it appears.',
          cancellable: false,
        },
        async () => {
          await resolved.cli.installDaemonElevated();
        },
      );
      this.output.appendLine('ssh_proxy daemon install completed; retrying proxy session.');
      vscode.window.showInformationMessage('ssh_proxy daemon installed. Retrying Remote Proxy.');
      await delay(1000);
      return true;
    } catch (installError) {
      const message = installError instanceof Error ? installError.message : String(installError);
      if (isSshProxyDaemonInstallCancelledMessage(message)) {
        this.output.appendLine('ssh_proxy daemon install was cancelled before completion.');
        void vscode.window.showWarningMessage('ssh_proxy daemon install was cancelled.');
        return false;
      }
      this.output.appendLine(`ssh_proxy daemon install failed: ${message}`);
      void vscode.window.showErrorMessage(`ssh_proxy daemon install failed: ${message}`, 'Show Output')
        .then((action) => {
          if (action === 'Show Output') {
            this.output.show(true);
          }
        });
      return false;
    }
  }

  public async diagnose(): Promise<void> {
    const snapshot = await this.collectStatusSnapshot();
    this.output.appendLine(REMOTE_PROXY_DIAGNOSTICS_HEADER);
    this.output.appendLine(snapshot.lines.join('\n'));

    if (this.forwarder === this.sshProxyBackend) {
      try {
        await this.sshProxyBackend.refreshStatus();
        this.output.appendLine('Remote forward verification is owned by the ssh_proxy daemon; use the daemon job and remote_setup fields above.');
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        this.output.appendLine(`Daemon status refresh failed: ${message}`);
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
      restartBackoff: 'daemon-managed',
      proxy,
      kernelStatus,
      lastError: this.forwarder.lastError,
    });
    return {
      lines,
      detectedHost,
      detectedSource,
      detectedConfidence,
      proxy,
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
    this.sshProxyBackend.dispose();
    this.statusBar.dispose();
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
      await this.monitorHealth(config);
      return;
    }

    const expectedHost = await this.resolveSshHost(config, false);
    const currentHost = this.forwarder.currentSshHost;
    if (!expectedHost || !currentHost || expectedHost === currentHost) {
      await this.monitorHealth(config);
      return;
    }

    this.output.appendLine(`Detected Remote SSH host change: ${currentHost} -> ${expectedHost}. Restarting proxy.`);
    await this.stopForwarder();
    await this.start({ interactive: false });
  }

  private async monitorHealth(config: RemoteProxyConfig): Promise<void> {
    if (!config.healthCheckEnabled || this.forwarder.status !== 'running') {
      return;
    }

    const now = Date.now();
    const healthIntervalMs = healthCheckIntervalMs(config.healthCheckIntervalSeconds);
    if (!shouldRunTimedCheck(now, this.lastHealthCheckAt, healthIntervalMs)) {
      return;
    }
    this.lastHealthCheckAt = now;

    try {
      if (this.forwarder === this.sshProxyBackend) {
        await this.sshProxyBackend.refreshStatus();
      }
      if (this.healthFailures > 0) {
        this.output.appendLine(`Daemon health status recovered after ${this.healthFailures} failed refresh attempt(s).`);
      }
      this.healthFailures = 0;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      const decision = recordHealthCheckFailure(this.healthFailures, config.healthCheckFailureThreshold);
      this.healthFailures = decision.failures;
      this.output.appendLine(`Daemon health status refresh failed (${decision.failures}/${decision.threshold}): ${message}`);
      if (!decision.shouldRestart) {
        return;
      }
      this.forwarder.fail(message);
      this.healthFailures = 0;
    }
  }

  private async stopForwarder(): Promise<void> {
    this.forwarder.stop();
  }

  private getEffectiveStatus(): string {
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
      sshHost: this.forwarder.currentSshHost,
      proxy: this.forwarder.appliedProxy,
      kernelStatus: this.getKernelStatus(),
      lastError: this.forwarder.lastError,
    }, text);
  }

}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
