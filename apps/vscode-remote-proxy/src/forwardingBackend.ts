import * as vscode from 'vscode';
import { AppliedProxy, RemoteProxyConfig } from './types';

export type ForwardingBackendStatus = 'stopped' | 'starting' | 'running' | 'failed';

export interface ForwardingBackend {
  readonly status: ForwardingBackendStatus;
  readonly lastError: string | undefined;
  readonly appliedProxy: AppliedProxy | undefined;
  readonly currentSshHost: string | undefined;
  readonly onDidChange: (listener: () => void) => vscode.Disposable;
  fail(message: string): void;
  start(config: RemoteProxyConfig, sshHost: string, proxy: AppliedProxy): Promise<void> | void;
  adoptShared(sshHost: string, proxy: AppliedProxy): void;
  startAndWait(config: RemoteProxyConfig, sshHost: string, proxy: AppliedProxy, waitMs: number): Promise<void>;
  stop(clearIntent?: boolean): void;
  dispose(): void;
}
