import { ChildProcessByStdio, spawn } from 'child_process';
import { Readable } from 'stream';
import * as vscode from 'vscode';
import { ForwardingBackend, ForwardingBackendStatus } from './forwardingBackend';
import { AppliedProxy, RemoteProxyConfig } from './types';

export class ForwardStartError extends Error {
  public constructor(message: string, public readonly deterministic: boolean) {
    super(message);
  }
}

export class OpenSshReverseBackend implements ForwardingBackend {
  private child: ChildProcessByStdio<null, Readable, Readable> | undefined;
  private retryTimer: NodeJS.Timeout | undefined;
  private intended = false;
  private current: AppliedProxy | undefined;
  private currentSshHostValue: string | undefined;
  private generation = 0;
  private statusValue: ForwardingBackendStatus = 'stopped';
  private lastErrorValue: string | undefined;
  private lastStderrValue = '';
  private readonly changeEmitter = new vscode.EventEmitter<void>();

  public readonly onDidChange = this.changeEmitter.event;

  public constructor(private readonly output: vscode.OutputChannel) {}

  public get status(): ForwardingBackendStatus {
    return this.statusValue;
  }

  public get lastError(): string | undefined {
    return this.lastErrorValue;
  }

  public get appliedProxy(): AppliedProxy | undefined {
    return this.current;
  }

  public get currentSshHost(): string | undefined {
    return this.currentSshHostValue;
  }

  public fail(message: string): void {
    this.lastErrorValue = message;
    this.statusValue = 'failed';
    this.changeEmitter.fire();
  }

  public start(config: RemoteProxyConfig, sshHost: string, proxy: AppliedProxy): void {
    this.startProcess(config, sshHost, proxy);
  }

  public adoptShared(sshHost: string, proxy: AppliedProxy): void {
    this.stop(false);
    this.generation += 1;
    this.intended = false;
    this.current = proxy;
    this.currentSshHostValue = sshHost;
    this.lastErrorValue = undefined;
    this.statusValue = 'running';
    this.changeEmitter.fire();
  }

  public startAndWait(config: RemoteProxyConfig, sshHost: string, proxy: AppliedProxy, waitMs: number): Promise<void> {
    const generation = this.startProcess(config, sshHost, proxy);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        disposable.dispose();
        if (generation === this.generation && this.statusValue === 'running') {
          resolve();
        } else {
          reject(new ForwardStartError(this.lastErrorValue ?? 'ssh forward did not reach running state', false));
        }
      }, waitMs);

      const disposable = this.onDidChange(() => {
        if (generation !== this.generation) {
          clearTimeout(timer);
          disposable.dispose();
          reject(new ForwardStartError('ssh forward was superseded by another start', false));
          return;
        }

        if (this.statusValue === 'failed') {
          clearTimeout(timer);
          disposable.dispose();
          const message = this.lastErrorValue ?? 'ssh forward failed';
          reject(new ForwardStartError(message, isDeterministicForwardFailure(message)));
        }
      });
    });
  }

  public stop(clearIntent = true): void {
    if (clearIntent) {
      this.intended = false;
      this.generation += 1;
    }

    if (this.retryTimer) {
      clearTimeout(this.retryTimer);
      this.retryTimer = undefined;
    }

    if (this.child) {
      const child = this.child;
      this.child = undefined;
      child.kill();
    }

    if (clearIntent) {
      this.statusValue = 'stopped';
      this.currentSshHostValue = undefined;
      this.changeEmitter.fire();
    }
  }

  public dispose(): void {
    this.stop();
    this.changeEmitter.dispose();
  }

  private startProcess(config: RemoteProxyConfig, sshHost: string, proxy: AppliedProxy): number {
    this.stop(false);
    const generation = ++this.generation;
    this.intended = true;
    this.current = proxy;
    this.currentSshHostValue = sshHost;
    this.lastErrorValue = undefined;
    this.lastStderrValue = '';
    this.statusValue = 'starting';
    this.changeEmitter.fire();

    const args = this.buildArgs(config, sshHost, proxy);
    this.output.appendLine(`Starting OpenSSH reverse proxy: ${config.sshExecutable} ${redactArgs(args).join(' ')}`);

    const child = spawn(config.sshExecutable, args, {
      windowsHide: true,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    this.child = child;

    child.stdout.on('data', (chunk: Buffer) => this.output.append(chunk.toString()));
    child.stderr.on('data', (chunk: Buffer) => {
      const text = chunk.toString();
      this.lastStderrValue += text;
      this.output.append(text);
    });
    child.once('spawn', () => {
      if (generation !== this.generation) {
        return;
      }
      this.statusValue = 'running';
      this.changeEmitter.fire();
    });
    child.once('error', (error) => {
      if (generation !== this.generation) {
        return;
      }
      this.lastErrorValue = error.message;
      this.statusValue = 'failed';
      this.changeEmitter.fire();
    });
    child.once('exit', (code, signal) => {
      if (generation !== this.generation) {
        return;
      }
      this.output.appendLine(`OpenSSH reverse proxy exited: code=${code ?? 'null'} signal=${signal ?? 'null'}`);
      this.child = undefined;

      if (!this.intended) {
        this.statusValue = 'stopped';
        this.changeEmitter.fire();
        return;
      }

      this.statusValue = 'failed';
      this.lastErrorValue = formatExitError(code, signal, this.lastStderrValue);
      this.changeEmitter.fire();

      if (config.retryOnExit && !isDeterministicForwardFailure(this.lastErrorValue)) {
        const retryDelayMs = Math.max(1, config.retryDelaySeconds) * 1000;
        this.retryTimer = setTimeout(() => this.start(config, sshHost, proxy), retryDelayMs);
      }
    });

    return generation;
  }

  private buildArgs(config: RemoteProxyConfig, sshHost: string, proxy: AppliedProxy): string[] {
    const args: string[] = [...config.sshArgs];
    if (config.sshBatchMode) {
      args.push('-o', 'BatchMode=yes');
    }
    args.push(
      '-o',
      `ConnectTimeout=${config.sshConnectTimeout}`,
      '-o',
      'ExitOnForwardFailure=yes',
      '-o',
      `ServerAliveInterval=${Math.max(1, config.sshServerAliveInterval)}`,
      '-o',
      `ServerAliveCountMax=${Math.max(1, config.sshServerAliveCountMax)}`,
      '-o',
      `TCPKeepAlive=${config.sshTcpKeepAlive ? 'yes' : 'no'}`,
      '-N',
      '-T',
      '-R',
      `${proxy.remoteBindHost}:${proxy.remotePort}:${proxy.local.host}:${proxy.local.port}`,
      sshHost,
    );
    return args;
  }
}

function redactArgs(args: readonly string[]): string[] {
  return args.map((arg) => arg.replace(/\/\/([^:@/]+):([^@/]+)@/, '//***:***@'));
}

function formatExitError(code: number | null, signal: NodeJS.Signals | null, stderr: string): string {
  const detail = stderr.trim();
  const suffix = detail ? `: ${detail}` : '';
  return `ssh exited with code ${code ?? 'null'} signal ${signal ?? 'null'}${suffix}`;
}

function isDeterministicForwardFailure(message: string): boolean {
  return /remote port forwarding failed|cannot listen to port|address already in use|administratively prohibited/i.test(message);
}
