import { spawn } from 'child_process';
import * as vscode from 'vscode';
import { RemoteProxyConfig } from './types';
import { findAvailableSshProxyCli, SshProxyCli } from './sshProxyCli';
import { resolveSshProxyExecutableCandidates, sshProxyUnavailableCandidatesMessage } from './sshProxyDiscovery';
import {
  assertSshProxyHostExecSucceeded,
  buildOpenSshRemoteScriptArgs,
  buildSshProxyHostExecHostArgs,
  sshProxyHostExecStdout,
  sshProxyHostExecTimeoutSecs,
} from './remoteCommandRunnerUtils';
import { RemoteCommandRunnerKind } from './remoteSetupRunnerPolicy';

export interface RemoteCommandRunner {
  readonly kind: RemoteCommandRunnerKind;
  runScript(config: RemoteProxyConfig, sshHost: string, script: string, label: string): Promise<void>;
  runScriptCapture(config: RemoteProxyConfig, sshHost: string, script: string, label: string): Promise<string>;
}

export class OpenSshCommandRunner implements RemoteCommandRunner {
  public readonly kind: RemoteCommandRunnerKind = 'openssh';

  public constructor(private readonly output: vscode.OutputChannel) {}

  public async runScript(config: RemoteProxyConfig, sshHost: string, script: string, label: string): Promise<void> {
    await this.runOpenSshScript(config, sshHost, script, label, false);
  }

  public async runScriptCapture(config: RemoteProxyConfig, sshHost: string, script: string, label: string): Promise<string> {
    return this.runOpenSshScript(config, sshHost, script, label, true);
  }

  private runOpenSshScript(
    config: RemoteProxyConfig,
    sshHost: string,
    script: string,
    label: string,
    captureStdout: boolean,
  ): Promise<string> {
    return new Promise((resolve, reject) => {
      const args = buildOpenSshRemoteScriptArgs(config, sshHost);
      this.output.appendLine(`Running remote setup through OpenSSH: ${label}`);
      const child = spawn(config.sshExecutable, args, {
        windowsHide: true,
        stdio: ['pipe', 'pipe', 'pipe'],
      });

      let stdout = '';
      let stderr = '';
      child.stdout.on('data', (chunk: Buffer) => {
        const text = chunk.toString();
        stdout += text;
        if (!captureStdout) {
          this.output.append(text);
        }
      });
      child.stderr.on('data', (chunk: Buffer) => {
        const text = chunk.toString();
        stderr += text;
        this.output.append(text);
      });
      child.once('error', (error) => {
        reject(new Error(`${label} failed to start OpenSSH: ${error.message}`));
      });
      child.once('exit', (code, signal) => {
        if (code === 0) {
          resolve(stdout);
          return;
        }
        reject(new Error(`${label} failed: code=${code ?? 'null'} signal=${signal ?? 'null'} ${stderr.trim()}`.trim()));
      });
      child.stdin.end(script);
    });
  }
}

export class SshProxyHostExecRunner implements RemoteCommandRunner {
  public readonly kind: RemoteCommandRunnerKind = 'ssh_proxy_host_exec';
  private cli: SshProxyCli | undefined;

  public constructor(
    private readonly output: vscode.OutputChannel,
    private readonly executable: string,
    private readonly extensionPath?: string,
  ) {
  }

  public async runScript(config: RemoteProxyConfig, sshHost: string, script: string, label: string): Promise<void> {
    const cli = await this.availableCli();
    const result = await cli.hostExecJson(
      sshHost,
      buildSshProxyHostExecHostArgs(config),
      script,
      label,
      sshProxyHostExecTimeoutSecs(config),
    );
    assertSshProxyHostExecSucceeded(result, label);
  }

  public async runScriptCapture(config: RemoteProxyConfig, sshHost: string, script: string, label: string): Promise<string> {
    const cli = await this.availableCli();
    const result = await cli.hostExecJson(
      sshHost,
      buildSshProxyHostExecHostArgs(config),
      script,
      label,
      sshProxyHostExecTimeoutSecs(config),
    );
    return sshProxyHostExecStdout(result, label);
  }

  private async availableCli(): Promise<SshProxyCli> {
    if (this.cli) {
      return this.cli;
    }
    const resolved = await findAvailableSshProxyCli(
      this.executable,
      this.output,
      { extensionPath: this.extensionPath },
    );
    if (resolved) {
      this.cli = resolved.cli;
      return resolved.cli;
    }
    throw new Error(sshProxyUnavailableCandidatesMessage(resolveSshProxyExecutableCandidates(
      this.executable,
      { extensionPath: this.extensionPath },
    )));
  }
}
