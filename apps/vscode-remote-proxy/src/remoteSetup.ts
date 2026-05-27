import * as vscode from 'vscode';
import { OpenSshCommandRunner, RemoteCommandRunner, SshProxyHostExecRunner } from './remoteCommandRunner';
import {
  createRemoteSetupFallbackRecord,
  formatRemoteSetupFallbackReason,
  preferredRemoteSetupRunnerKind,
  RemoteCommandRunnerKind,
  RemoteSetupFallbackRecord,
  shouldFallbackRemoteSetup,
} from './remoteSetupRunnerPolicy';
import {
  buildCleanupScript,
  buildGitConfigScript,
  buildProxyEnv,
  buildReadRemoteStatusFileScript,
  buildRemotePortFreeScript,
  buildRemoteSettingsScript,
  buildRemoteStatusFileScript,
  buildServerEnvSetupScript,
  buildVerifyForwardScript,
  ProxyEnvironment,
} from './remoteSetupScripts';
import { parseRemoteProxyStatusFile, RemoteProxyStatusFile } from './remoteStatusFile';
import { AppliedProxy, RemoteProxyConfig } from './types';

export class RemoteSetup {
  private readonly fallbackRecords: RemoteSetupFallbackRecord[] = [];

  public constructor(
    private readonly output: vscode.OutputChannel,
    private readonly extensionPath?: string,
  ) {}

  public get remoteSetupFallbacks(): readonly RemoteSetupFallbackRecord[] {
    return this.fallbackRecords;
  }

  public get lastFallbackReason(): string | undefined {
    const record = this.fallbackRecords[this.fallbackRecords.length - 1];
    return record ? formatRemoteSetupFallbackReason(record) : undefined;
  }

  public async applyAll(config: RemoteProxyConfig, sshHost: string, proxy: AppliedProxy): Promise<void> {
    if (config.applyVscodeSettings) {
      await this.applyVscodeApiSettings(config, proxy);
    }

    const tasks: Promise<void>[] = [];
    if (config.applyRemoteMachineSettings) {
      tasks.push(this.applyRemoteMachineSettings(config, sshHost, proxy));
    }
    if (config.applyServerEnvSetup) {
      tasks.push(this.applyServerEnvSetup(config, sshHost, proxy));
    }
    if (config.applyGitConfig) {
      tasks.push(this.applyGitConfig(config, sshHost, proxy));
    }
    if (config.applyRemoteStatusFile) {
      tasks.push(this.applyRemoteStatusFile(config, sshHost, proxy));
    }

    await Promise.all(tasks);
  }

  public async cleanupAll(config: RemoteProxyConfig, sshHost: string): Promise<void> {
    await this.runSshScript(
      config,
      sshHost,
      buildCleanupScript(getServerDirName(), getRemoteWorkspacePaths()),
      'cleanup remote proxy settings',
    );
  }

  public async verifyForward(config: RemoteProxyConfig, sshHost: string, proxy: AppliedProxy): Promise<void> {
    await this.runSshScript(
      config,
      sshHost,
      buildVerifyForwardScript(proxy.remoteBindHost, proxy.remotePort),
      `verify remote forwarded port ${proxy.remoteBindHost}:${proxy.remotePort}`,
    );
  }

  public async verifyForwardReady(
    config: RemoteProxyConfig,
    sshHost: string,
    proxy: AppliedProxy,
    options: { readonly timeoutMs?: number; readonly pollMs?: number } = {},
  ): Promise<void> {
    const timeoutMs = Math.max(1000, options.timeoutMs ?? 8000);
    const pollMs = Math.max(100, options.pollMs ?? 300);
    const deadline = Date.now() + timeoutMs;
    let attempt = 0;
    let lastError: unknown;

    while (Date.now() <= deadline) {
      attempt += 1;
      try {
        await this.verifyForward(config, sshHost, proxy);
        if (attempt > 1) {
          this.output.appendLine(`Remote forwarded port ${proxy.remoteBindHost}:${proxy.remotePort} became ready after ${attempt} attempts.`);
        }
        return;
      } catch (error) {
        lastError = error;
        if (Date.now() + pollMs > deadline) {
          break;
        }
        await sleep(pollMs);
      }
    }

    const message = lastError instanceof Error ? lastError.message : String(lastError);
    throw new Error(`remote forwarded port ${proxy.remoteBindHost}:${proxy.remotePort} did not become ready within ${timeoutMs} ms: ${message}`);
  }

  public async isRemotePortFree(config: RemoteProxyConfig, sshHost: string, host: string, port: number): Promise<boolean> {
    try {
      await this.runSshScript(config, sshHost, buildRemotePortFreeScript(host, port), `check remote port ${host}:${port}`);
      return true;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (/code=1\b/.test(message)) {
        return false;
      }
      throw error;
    }
  }

  public async readRemoteStatus(config: RemoteProxyConfig, sshHost: string): Promise<RemoteProxyStatusFile | undefined> {
    try {
      const raw = await this.runSshScriptCapture(
        config,
        sshHost,
        buildReadRemoteStatusFileScript(getServerDirName()),
        'read remote proxy status file',
      );
      return parseRemoteProxyStatusFile(raw);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      this.output.appendLine(`remote-proxy: failed to read remote status file: ${message}`);
      return undefined;
    }
  }

  public async applyVscodeApiSettings(config: RemoteProxyConfig, proxy: AppliedProxy): Promise<void> {
    const settings = vscode.workspace.getConfiguration();
    await settings.update('http.proxy', proxy.remoteUrl, vscode.ConfigurationTarget.Global);
    await settings.update('http.proxySupport', config.proxySupport, vscode.ConfigurationTarget.Global);

    if (config.applyTerminalEnv) {
      const env = buildProxyEnv(proxy.remoteUrl, config.noProxy);
      await mergeTerminalEnvSetting('terminal.integrated.env.linux', env);
      await mergeTerminalEnvSetting('terminal.integrated.env.osx', env);
      await mergeTerminalEnvSetting('terminal.integrated.env.windows', env);
    }
  }

  private async applyRemoteMachineSettings(config: RemoteProxyConfig, sshHost: string, proxy: AppliedProxy): Promise<void> {
    const env = config.applyTerminalEnv ? buildProxyEnv(proxy.remoteUrl, config.noProxy) : undefined;
    const payload = {
      serverDir: getServerDirName(),
      values: {
        'http.proxy': proxy.remoteUrl,
        'http.proxySupport': config.proxySupport,
        ...(env ? {
          'terminal.integrated.env.linux': env,
          'terminal.integrated.env.osx': env,
          'terminal.integrated.env.windows': env
        } : {})
      }
    };

    await this.runSshScript(config, sshHost, buildRemoteSettingsScript(payload), 'patch remote VS Code machine settings');
  }

  private async applyServerEnvSetup(config: RemoteProxyConfig, sshHost: string, proxy: AppliedProxy): Promise<void> {
    await this.runSshScript(
      config,
      sshHost,
      buildServerEnvSetupScript(getServerDirName(), buildProxyEnv(proxy.remoteUrl, config.noProxy)),
      'patch remote server-env-setup',
    );
  }

  private async applyGitConfig(config: RemoteProxyConfig, sshHost: string, proxy: AppliedProxy): Promise<void> {
    const script = buildGitConfigScript({
      proxyUrl: proxy.remoteUrl,
      workspacePaths: getRemoteWorkspacePaths(),
      applyGlobal: config.applyGitGlobalConfig,
      applyWorkspace: config.applyGitWorkspaceConfig,
      forceOverride: config.applyGitForceOverride
    });
    await this.runSshScript(config, sshHost, script, 'apply remote Git proxy config');
  }

  private async applyRemoteStatusFile(config: RemoteProxyConfig, sshHost: string, proxy: AppliedProxy): Promise<void> {
    const payload = {
      proxyUrl: proxy.remoteUrl,
      bindHost: proxy.remoteBindHost,
      port: proxy.remotePort,
      updatedAt: new Date().toISOString(),
      localProxySource: proxy.local.source,
      localProxyUrl: proxy.local.url,
      backend: proxy.backend ?? 'openssh',
      routeId: proxy.routeId,
      routeOwner: proxy.routeOwner,
      selectedTransport: proxy.selectedTransport,
      connectMode: proxy.connectMode,
      fallbackReason: proxy.fallbackReason,
    };
    await this.runSshScript(
      config,
      sshHost,
      buildRemoteStatusFileScript(getServerDirName(), payload),
      'write remote proxy status file',
    );
  }

  private runSshScript(config: RemoteProxyConfig, sshHost: string, script: string, label: string): Promise<void> {
    return this.runWithPreferredRunner(config, sshHost, script, label);
  }

  private runSshScriptCapture(config: RemoteProxyConfig, sshHost: string, script: string, label: string): Promise<string> {
    return this.runWithPreferredRunnerCapture(config, sshHost, script, label);
  }

  private async runWithPreferredRunner(config: RemoteProxyConfig, sshHost: string, script: string, label: string): Promise<void> {
    const preferred = await this.chooseRunner(config);
    try {
      await preferred.runScript(config, sshHost, script, label);
    } catch (error) {
      if (!shouldFallbackRemoteSetup(config.sshProxyRemoteSetup, preferred.kind)) {
        throw error;
      }
      const record = createRemoteSetupFallbackRecord(label, preferred.kind, 'openssh', error);
      this.fallbackRecords.push(record);
      this.output.appendLine(`Remote setup fallback: ${formatRemoteSetupFallbackReason(record)}`);
      await this.createRunner(config, 'openssh').runScript(config, sshHost, script, label);
    }
  }

  private async runWithPreferredRunnerCapture(config: RemoteProxyConfig, sshHost: string, script: string, label: string): Promise<string> {
    const preferred = await this.chooseRunner(config);
    try {
      return await preferred.runScriptCapture(config, sshHost, script, label);
    } catch (error) {
      if (!shouldFallbackRemoteSetup(config.sshProxyRemoteSetup, preferred.kind)) {
        throw error;
      }
      const record = createRemoteSetupFallbackRecord(label, preferred.kind, 'openssh', error);
      this.fallbackRecords.push(record);
      this.output.appendLine(`Remote setup fallback: ${formatRemoteSetupFallbackReason(record)}`);
      return this.createRunner(config, 'openssh').runScriptCapture(config, sshHost, script, label);
    }
  }

  private async chooseRunner(config: RemoteProxyConfig): Promise<RemoteCommandRunner> {
    return this.createRunner(config, preferredRemoteSetupRunnerKind(config.sshProxyRemoteSetup));
  }

  private createRunner(config: RemoteProxyConfig, kind: RemoteCommandRunnerKind): RemoteCommandRunner {
    if (kind === 'openssh') {
      return new OpenSshCommandRunner(this.output);
    }
    return new SshProxyHostExecRunner(this.output, config.sshProxyExecutable, this.extensionPath);
  }
}

async function mergeTerminalEnvSetting(key: string, env: ProxyEnvironment): Promise<void> {
  const config = vscode.workspace.getConfiguration();
  const current = config.get<Record<string, string>>(key, {});
  await config.update(key, { ...current, ...env }, vscode.ConfigurationTarget.Global);
}

function getServerDirName(): string {
  return vscode.env.appName.toLowerCase().includes('insider') ? '.vscode-server-insiders' : '.vscode-server';
}

function getRemoteWorkspacePaths(): string[] {
  const seen = new Set<string>();
  for (const folder of vscode.workspace.workspaceFolders ?? []) {
    if (folder.uri.scheme !== 'vscode-remote' || !folder.uri.path) {
      continue;
    }
    seen.add(folder.uri.path);
  }
  return [...seen];
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
