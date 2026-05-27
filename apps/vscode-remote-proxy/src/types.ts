export type LocalProxyMode = 'auto' | 'env' | 'manual';
export type ProxyScheme = 'http' | 'https' | 'socks4' | 'socks5';
export type SupportedRemoteKind = 'ssh' | 'wsl' | 'dev-container' | 'none' | 'other';
export type ForwardingBackendKind = 'ssh_proxy';
export type SshProxyConnectMode = 'auto' | 'reverse-link' | 'direct';

export interface LocalProxy {
  readonly url: string;
  readonly scheme: ProxyScheme;
  readonly host: string;
  readonly port: number;
  readonly source: string;
}

export interface RemoteContext {
  readonly kind: SupportedRemoteKind;
  readonly name: string | undefined;
  readonly authority: string | undefined;
  readonly sshHost: string | undefined;
  readonly sshHostSource: string | undefined;
}

export interface RemoteProxyConfig {
  readonly enabled: boolean;
  readonly autoStart: boolean;
  readonly localProxyMode: LocalProxyMode;
  readonly localProxyUrl: string;
  readonly localProxyHosts: readonly string[];
  readonly localProxyAutoPorts: readonly number[];
  readonly localProxyDefaultScheme: 'http' | 'socks5';
  readonly remotePort: number;
  readonly remoteAutoPickPort: boolean;
  readonly remotePortRangeSize: number;
  readonly remoteBindHost: string;
  readonly noProxy: string;
  readonly sshHostOverride: string;
  readonly sshUseStorageFallback: boolean;
  readonly sshProxyExecutable: string;
  readonly sshProxyConnectMode: SshProxyConnectMode;
  readonly restartOnHostChange: boolean;
  readonly verifyAfterStart: boolean;
  readonly healthCheckEnabled: boolean;
  readonly healthCheckIntervalSeconds: number;
  readonly healthCheckFailureThreshold: number;
  readonly applyVscodeSettings: boolean;
  readonly applyRemoteMachineSettings: boolean;
  readonly applyTerminalEnv: boolean;
  readonly applyGitConfig: boolean;
  readonly applyGitGlobalConfig: boolean;
  readonly applyGitWorkspaceConfig: boolean;
  readonly applyGitForceOverride: boolean;
  readonly applyServerEnvSetup: boolean;
  readonly applyRemoteStatusFile: boolean;
  readonly proxySupport: 'override' | 'on' | 'off' | 'fallback';
}

export interface HostProfile {
  readonly localProxyUrl?: string;
  readonly remotePort?: number;
  readonly remoteBindHost?: string;
  readonly noProxy?: string;
  readonly applyGitConfig?: boolean;
  readonly applyGitGlobalConfig?: boolean;
  readonly applyGitWorkspaceConfig?: boolean;
  readonly applyGitForceOverride?: boolean;
  readonly applyServerEnvSetup?: boolean;
}

export interface AppliedProxy {
  readonly local: LocalProxy;
  readonly remoteUrl: string;
  readonly remotePort: number;
  readonly remoteBindHost: string;
  readonly workspaceId?: string;
  readonly routeId?: string;
  readonly routeOwner?: string;
  readonly selectedTransport?: string;
  readonly connectMode?: string;
  readonly fallbackReason?: string;
  readonly backend?: ForwardingBackendKind;
  readonly cleanupCommand?: string;
}
