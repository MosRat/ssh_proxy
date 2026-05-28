import { spawn } from 'child_process';
import * as vscode from 'vscode';
import {
  buildSshProxyDaemonInstallArgs,
  buildSshProxyDownArgs,
  buildSshProxyVscodeApplySettingsArgs,
  buildSshProxyVscodeStatusArgs,
  buildSshProxyVscodeUpArgs,
  formatSshProxyCommand,
  normalizeSshProxyExecutable,
  parseSshProxyJson,
  summarizeSshProxyOutput,
} from './sshProxyCliUtils';
import {
  describeSshProxyDiscovery,
  resolveSshProxyExecutableCandidates,
  SshProxyExecutableDiscovery,
  SshProxyExecutableDiscoveryOptions,
} from './sshProxyDiscovery';
import { SshTargetConfig } from './types';

export interface CommandResult {
  readonly exitCode: number;
  readonly stdout: string;
  readonly stderr: string;
}

export interface SshProxyRunOptions {
  readonly label?: string;
  readonly timeoutMs?: number;
}

export interface AvailableSshProxyCli {
  readonly cli: SshProxyCli;
  readonly discovery: SshProxyExecutableDiscovery;
  readonly candidates: readonly SshProxyExecutableDiscovery[];
}

const DEFAULT_RUN_TIMEOUT_MS = 30_000;
const AVAILABLE_TIMEOUT_MS = 3_000;
const ROUTE_TIMEOUT_MS = 60_000;
const STOP_ROUTE_TIMEOUT_MS = 15_000;
const ROUTES_STATUS_TIMEOUT_MS = 10_000;
const DAEMON_INSTALL_TIMEOUT_MS = 180_000;

export class SshProxyCli {
  private readonly executable: string;

  public constructor(
    executable: string,
    private readonly output: vscode.OutputChannel,
  ) {
    this.executable = normalizeSshProxyExecutable(executable);
  }

  public async available(): Promise<boolean> {
    try {
      await this.run(['--version'], undefined, { label: 'ssh_proxy --version', timeoutMs: AVAILABLE_TIMEOUT_MS });
      return true;
    } catch {
      return false;
    }
  }

  public async vscodeUpJson(options: {
    readonly target: string;
    readonly workspace: string;
    readonly localProxy: string;
    readonly remoteBind: string;
    readonly remotePort: number;
    readonly connectMode: 'auto' | 'reverse-link' | 'direct';
    readonly sshTarget?: SshTargetConfig;
    readonly workspacePaths?: readonly string[];
    readonly serverDir?: string;
    readonly noProxy?: string;
    readonly proxySupport?: string;
    readonly applyRemoteMachineSettings?: boolean;
    readonly applyTerminalEnv?: boolean;
    readonly applyServerEnvSetup?: boolean;
    readonly applyGitConfig?: boolean;
    readonly applyGitGlobalConfig?: boolean;
    readonly applyGitWorkspaceConfig?: boolean;
    readonly applyGitForceOverride?: boolean;
    readonly applyRemoteStatusFile?: boolean;
    readonly verifyRemotePort?: boolean;
  }): Promise<unknown> {
    return this.runJson(buildSshProxyVscodeUpArgs(options), undefined, {
      label: 'ssh_proxy vscode up',
      timeoutMs: ROUTE_TIMEOUT_MS,
    });
  }

  public async vscodeStatusJson(options: {
    readonly workspace?: string;
    readonly target?: string;
  }): Promise<unknown> {
    return this.runJson(buildSshProxyVscodeStatusArgs(options), undefined, {
      label: 'ssh_proxy vscode status',
      timeoutMs: ROUTES_STATUS_TIMEOUT_MS,
    });
  }

  public async vscodeApplySettingsJson(options: {
    readonly target: string;
    readonly workspace: string;
    readonly proxyUrl: string;
  }): Promise<unknown> {
    return this.runJson(buildSshProxyVscodeApplySettingsArgs(options), undefined, {
      label: 'ssh_proxy vscode apply-settings',
      timeoutMs: ROUTE_TIMEOUT_MS,
    });
  }

  public async downJson(options: {
    readonly routeId?: string;
    readonly workspace?: string;
    readonly target?: string;
  }): Promise<unknown> {
    return this.runJson(buildSshProxyDownArgs(options), undefined, {
      label: 'ssh_proxy down',
      timeoutMs: STOP_ROUTE_TIMEOUT_MS,
    });
  }

  public async installDaemonElevated(): Promise<CommandResult> {
    return this.run(buildSshProxyDaemonInstallArgs({ scope: 'system', elevate: true }), undefined, {
      label: 'ssh_proxy daemon install',
      timeoutMs: DAEMON_INSTALL_TIMEOUT_MS,
    });
  }

  public async run(args: readonly string[], input?: string, options: SshProxyRunOptions = {}): Promise<CommandResult> {
    return new Promise((resolve, reject) => {
      const timeoutMs = options.timeoutMs ?? DEFAULT_RUN_TIMEOUT_MS;
      const label = options.label ?? 'ssh_proxy command';
      const command = formatSshProxyCommand(this.executable, args);
      let settled = false;
      let timedOut = false;
      let timer: NodeJS.Timeout | undefined;

      const finish = (error: Error | undefined, result?: CommandResult): void => {
        if (settled) {
          return;
        }
        settled = true;
        if (timer) {
          clearTimeout(timer);
        }
        if (error) {
          reject(error);
          return;
        }
        resolve(result ?? { exitCode: 0, stdout: '', stderr: '' });
      };

      this.output.appendLine(command);
      const child = spawn(this.executable, [...args], {
        windowsHide: true,
        stdio: ['pipe', 'pipe', 'pipe'],
      });
      let stdout = '';
      let stderr = '';
      child.stdout.on('data', (chunk: Buffer) => {
        stdout += chunk.toString();
      });
      child.stderr.on('data', (chunk: Buffer) => {
        stderr += chunk.toString();
      });

      if (timeoutMs > 0) {
        timer = setTimeout(() => {
          timedOut = true;
          child.kill();
          finish(new Error(`${label} timed out after ${timeoutMs} ms: ${command}`));
        }, timeoutMs);
        timer.unref?.();
      }

      child.once('error', (error) => {
        finish(new Error(`${label} failed to start: ${error.message}: ${command}`));
      });
      child.once('exit', (code, signal) => {
        if (settled) {
          return;
        }
        if (timedOut) {
          finish(new Error(`${label} timed out after ${timeoutMs} ms: ${command}`));
          return;
        }
        const exitCode = code ?? (signal ? 128 : 0);
        if (exitCode !== 0) {
          const stderrSummary = summarizeSshProxyOutput(stderr);
          const stdoutSummary = summarizeSshProxyOutput(stdout);
          const detail = [stderrSummary, stdoutSummary ? `stdout: ${stdoutSummary}` : '']
            .filter(Boolean)
            .join('\n');
          finish(new Error(`${label} failed with code ${code ?? 'null'} signal ${signal ?? 'null'}: ${command}${detail ? `\n${detail}` : ''}`));
          return;
        }
        finish(undefined, { exitCode, stdout, stderr });
      });
      child.stdin.once('error', (error: NodeJS.ErrnoException) => {
        if (!settled && error.code !== 'EPIPE') {
          this.output.appendLine(`${label} stdin error: ${error.message}`);
        }
      });
      if (input !== undefined) {
        child.stdin.end(input, 'utf8');
      } else {
        child.stdin.end();
      }
    });
  }

  public async runJson(args: readonly string[], input?: string, options: SshProxyRunOptions = {}): Promise<unknown> {
    const result = await this.run(args, input, options);
    return parseSshProxyJson(result.stdout, options.label ?? 'ssh_proxy command');
  }
}

export async function findAvailableSshProxyCli(
  configured: string,
  output: vscode.OutputChannel,
  options: SshProxyExecutableDiscoveryOptions = {},
): Promise<AvailableSshProxyCli | undefined> {
  const candidates = resolveSshProxyExecutableCandidates(configured, options);
  for (const discovery of candidates) {
    output.appendLine(`Checking ${describeSshProxyDiscovery(discovery)}`);
    const cli = new SshProxyCli(discovery.executable, output);
    if (await cli.available()) {
      return { cli, discovery, candidates };
    }
  }
  return undefined;
}

