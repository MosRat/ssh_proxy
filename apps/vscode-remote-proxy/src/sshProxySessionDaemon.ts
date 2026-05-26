import { ChildProcess } from 'child_process';
import { randomBytes } from 'crypto';
import * as net from 'net';
import * as vscode from 'vscode';
import { SshProxyCli } from './sshProxyCli';
import { SshProxyControlConnection, summarizeSshProxyOutput } from './sshProxyCliUtils';

export interface SshProxySessionDaemon extends SshProxyControlConnection {
  readonly transport: string;
  readonly child: ChildProcess;
  readonly startedAt: number;
}

const READY_TIMEOUT_MS = 7_500;
const READY_POLL_MS = 250;
const START_ATTEMPTS = 3;
const OUTPUT_LIMIT = 8192;

export async function startSshProxySessionDaemon(
  cli: SshProxyCli,
  output: vscode.OutputChannel,
  reason: string,
): Promise<SshProxySessionDaemon> {
  let lastError: unknown;
  for (let attempt = 1; attempt <= START_ATTEMPTS; attempt += 1) {
    const token = randomBytes(32).toString('hex');
    const controlPort = await pickLoopbackPort();
    const transportPort = await pickLoopbackPort();
    const endpoint = `tcp://127.0.0.1:${controlPort}`;
    const transport = `127.0.0.1:${transportPort}`;
    const child = cli.startNodeDaemon({
      control: endpoint,
      transport,
      token,
      name: 'vscode-remote-proxy-session',
    });
    const daemon: SshProxySessionDaemon = {
      endpoint,
      token,
      transport,
      child,
      startedAt: Date.now(),
    };
    const outputBuffer = collectDaemonOutput(child, token);
    output.appendLine(`ssh_proxy session daemon starting because ${reason}; endpoint=${endpoint}, transport=${transport}, attempt=${attempt}`);

    try {
      await waitForSessionDaemonReady(cli, daemon, outputBuffer);
      output.appendLine(`ssh_proxy session daemon ready at ${endpoint}`);
      return daemon;
    } catch (error) {
      lastError = error;
      output.appendLine(`ssh_proxy session daemon attempt ${attempt} failed: ${error instanceof Error ? error.message : String(error)}`);
      await shutdownSshProxySessionDaemon(cli, daemon, output);
    }
  }
  throw new Error(`ssh_proxy session daemon could not start after ${START_ATTEMPTS} attempts: ${lastError instanceof Error ? lastError.message : String(lastError)}`);
}

export async function shutdownSshProxySessionDaemon(
  cli: SshProxyCli,
  daemon: SshProxySessionDaemon,
  output: vscode.OutputChannel,
): Promise<void> {
  try {
    await cli.nodeControlShutdownJson(daemon);
    output.appendLine(`ssh_proxy session daemon shutdown requested at ${daemon.endpoint}`);
  } catch (error) {
    output.appendLine(`ssh_proxy session daemon shutdown request failed: ${error instanceof Error ? error.message : String(error)}`);
  }

  if (daemon.child.exitCode === null && daemon.child.signalCode === null) {
    daemon.child.kill();
  }
}

async function waitForSessionDaemonReady(
  cli: SshProxyCli,
  daemon: SshProxySessionDaemon,
  outputBuffer: () => string,
): Promise<void> {
  const deadline = Date.now() + READY_TIMEOUT_MS;
  let lastError: unknown;
  while (Date.now() < deadline) {
    if (daemon.child.exitCode !== null || daemon.child.signalCode !== null) {
      throw new Error(`daemon exited before ready: code=${daemon.child.exitCode ?? 'null'} signal=${daemon.child.signalCode ?? 'null'}${formatDaemonOutput(outputBuffer())}`);
    }
    try {
      const status = await cli.nodeControlStatusJson(daemon);
      const record = asRecord(status);
      if (record?.ok !== false) {
        return;
      }
      lastError = new Error(`status returned ok=false${formatDaemonOutput(outputBuffer())}`);
    } catch (error) {
      lastError = error;
    }
    await sleep(READY_POLL_MS);
  }
  throw new Error(`daemon did not become ready within ${READY_TIMEOUT_MS} ms: ${lastError instanceof Error ? lastError.message : String(lastError)}${formatDaemonOutput(outputBuffer())}`);
}

function collectDaemonOutput(child: ChildProcess, token: string): () => string {
  let output = '';
  const append = (chunk: Buffer): void => {
    output = `${output}${chunk.toString()}`;
    if (output.length > OUTPUT_LIMIT) {
      output = output.slice(output.length - OUTPUT_LIMIT);
    }
  };
  child.stdout?.on('data', append);
  child.stderr?.on('data', append);
  child.once('error', (error) => {
    append(Buffer.from(`\nprocess error: ${error.message}\n`));
  });
  return () => summarizeSshProxyOutput(redactToken(output, token), OUTPUT_LIMIT);
}

function formatDaemonOutput(output: string): string {
  return output ? `\n${output}` : '';
}

function redactToken(text: string, token: string): string {
  return token ? text.split(token).join('<redacted>') : text;
}

function pickLoopbackPort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      const port = typeof address === 'object' && address ? address.port : undefined;
      server.close((error) => {
        if (error) {
          reject(error);
          return;
        }
        if (!port) {
          reject(new Error('failed to reserve a loopback port'));
          return;
        }
        resolve(port);
      });
    });
  });
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
}
