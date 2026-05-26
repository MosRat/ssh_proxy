import { ChildProcess, spawn } from 'child_process';
import * as vscode from 'vscode';
import {
  buildSshProxyNodeControlShutdownArgs,
  buildSshProxyNodeControlStatusArgs,
  buildSshProxyNodeDaemonArgs,
  buildSshProxyServiceStatusArgs,
  buildSshProxyRoutesArgs,
  buildSshProxyStopRouteArgs,
  formatSshProxyCommand,
  normalizeSshProxyExecutable,
  parseSshProxyJson,
  SshProxyControlConnection,
  summarizeSshProxyOutput,
} from './sshProxyCliUtils';
import {
  describeSshProxyDiscovery,
  resolveSshProxyExecutableCandidates,
  SshProxyExecutableDiscovery,
  SshProxyExecutableDiscoveryOptions,
} from './sshProxyDiscovery';

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
const SERVICE_STATUS_TIMEOUT_MS = 5_000;
const SERVICE_INSTALL_TIMEOUT_MS = 60_000;
const ROUTE_TIMEOUT_MS = 60_000;
const STOP_ROUTE_TIMEOUT_MS = 15_000;
const ROUTES_STATUS_TIMEOUT_MS = 10_000;

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

  public async serviceStatusJson(): Promise<unknown> {
    return this.runJson(buildSshProxyServiceStatusArgs(), undefined, {
      label: 'ssh_proxy service status',
      timeoutMs: SERVICE_STATUS_TIMEOUT_MS,
    });
  }

  public async serviceInstall(scope: 'user' | 'system' = 'user'): Promise<void> {
    await this.run(['service', '--scope', scope, 'install'], undefined, {
      label: 'ssh_proxy service install',
      timeoutMs: SERVICE_INSTALL_TIMEOUT_MS,
    });
  }

  public async routeExplainJson(args: readonly string[]): Promise<unknown> {
    return this.runJson(['route', ...args, '--explain', '--json'], undefined, {
      label: 'ssh_proxy route explain',
      timeoutMs: ROUTE_TIMEOUT_MS,
    });
  }

  public async routeStartJson(args: readonly string[]): Promise<unknown> {
    return this.runJson(['route', ...args, '--json'], undefined, {
      label: 'ssh_proxy route start',
      timeoutMs: ROUTE_TIMEOUT_MS,
    });
  }

  public async nodeControlStatusJson(connection: SshProxyControlConnection = {}): Promise<unknown> {
    return this.runJson(buildSshProxyNodeControlStatusArgs(connection), undefined, {
      label: 'ssh_proxy node status',
      timeoutMs: ROUTES_STATUS_TIMEOUT_MS,
    });
  }

  public async nodeControlShutdownJson(connection: SshProxyControlConnection = {}): Promise<unknown> {
    return this.runJson(buildSshProxyNodeControlShutdownArgs(connection), undefined, {
      label: 'ssh_proxy node shutdown',
      timeoutMs: STOP_ROUTE_TIMEOUT_MS,
    });
  }

  public startNodeDaemon(options: {
    readonly control: string;
    readonly transport: string;
    readonly token: string;
    readonly name?: string;
  }): ChildProcess {
    const args = buildSshProxyNodeDaemonArgs(options);
    this.output.appendLine(formatSshProxyCommand(this.executable, args));
    return spawn(this.executable, args, {
      windowsHide: true,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
  }

  public async stopRouteJson(id: string, connection: SshProxyControlConnection = {}): Promise<unknown> {
    return this.runJson(buildSshProxyStopRouteArgs(id, connection), undefined, {
      label: 'ssh_proxy stop route',
      timeoutMs: STOP_ROUTE_TIMEOUT_MS,
    });
  }

  public async routesJson(connection: SshProxyControlConnection = {}): Promise<unknown> {
    return this.runJson(buildSshProxyRoutesArgs(connection), undefined, {
      label: 'ssh_proxy node routes',
      timeoutMs: ROUTES_STATUS_TIMEOUT_MS,
    });
  }

  public async hostExecJson(
    host: string,
    hostArgs: readonly string[],
    script: string,
    label: string,
    timeoutSecs: number,
  ): Promise<unknown> {
    const timeoutMs = hostExecTimeoutMs(timeoutSecs);
    return this.runJson(
      ['host', host, ...hostArgs, 'exec', '--stdin', '--label', label, '--timeout-secs', String(timeoutSecs), '--json'],
      script,
      { label: `ssh_proxy host exec ${label}`, timeoutMs },
    );
  }

  public async hostExec(host: string, hostArgs: readonly string[], script: string, label: string, timeoutSecs: number): Promise<CommandResult> {
    return this.run(
      ['host', host, ...hostArgs, 'exec', '--stdin', '--label', label, '--timeout-secs', String(timeoutSecs), '--json'],
      script,
      { label: `ssh_proxy host exec ${label}`, timeoutMs: hostExecTimeoutMs(timeoutSecs) },
    );
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

function hostExecTimeoutMs(timeoutSecs: number): number {
  return Math.max(5, timeoutSecs + 5) * 1000;
}
