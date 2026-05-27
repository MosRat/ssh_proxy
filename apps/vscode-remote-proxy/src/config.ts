import * as vscode from 'vscode';
import { HostProfile, RemoteProxyConfig } from './types';

export function readConfig(): RemoteProxyConfig {
  const config = vscode.workspace.getConfiguration('remoteProxy');

  return {
    enabled: config.get<boolean>('enabled', true),
    autoStart: config.get<boolean>('autoStart', true),
    backend: config.get<'auto' | 'ssh_proxy' | 'openssh'>('backend', 'auto'),
    localProxyMode: config.get<'auto' | 'env' | 'manual'>('localProxy.mode', 'auto'),
    localProxyUrl: config.get<string>('localProxy.url', ''),
    localProxyHosts: config.get<readonly string[]>('localProxy.hosts', ['127.0.0.1', 'localhost']),
    localProxyAutoPorts: config.get<readonly number[]>('localProxy.autoPorts', [7890, 7897, 1080, 8080, 3128, 6152]),
    localProxyDefaultScheme: config.get<'http' | 'socks5'>('localProxy.defaultScheme', 'http'),
    remotePort: config.get<number>('remote.port', 17890),
    remoteAutoPickPort: config.get<boolean>('remote.autoPickPort', true),
    remotePortRangeSize: config.get<number>('remote.portRangeSize', 20),
    remoteBindHost: config.get<string>('remote.bindHost', '127.0.0.1'),
    noProxy: config.get<string>('noProxy', 'localhost,127.0.0.1,::1'),
    sshExecutable: config.get<string>('ssh.executable', 'ssh'),
    sshHostOverride: config.get<string>('ssh.host', ''),
    sshUseStorageFallback: config.get<boolean>('ssh.useStorageFallback', false),
    sshArgs: config.get<readonly string[]>('ssh.args', []),
    sshProxyExecutable: config.get<string>('sshProxy.executable', 'ssh_proxy'),
    sshProxyAutoInstallLocalService: config.get<boolean>('sshProxy.autoInstallLocalService', true),
    sshProxyAllowElevationPrompt: config.get<boolean>('sshProxy.allowElevationPrompt', true),
    sshProxyPreferPersistentService: config.get<boolean>('sshProxy.preferPersistentService', true),
    sshProxyConnectMode: config.get<'auto' | 'reverse-link' | 'direct'>('sshProxy.connectMode', 'reverse-link'),
    sshProxyRouteVolatile: config.get<boolean>('sshProxy.routeVolatile', true),
    sshProxyRemoteSetup: config.get<'auto' | 'ssh_proxy' | 'openssh'>('sshProxy.remoteSetup', 'auto'),
    sshBatchMode: config.get<boolean>('ssh.batchMode', true),
    sshConnectTimeout: config.get<number>('ssh.connectTimeout', 12),
    sshServerAliveInterval: config.get<number>('ssh.serverAliveInterval', 60),
    sshServerAliveCountMax: config.get<number>('ssh.serverAliveCountMax', 3),
    sshTcpKeepAlive: config.get<boolean>('ssh.tcpKeepAlive', true),
    retryOnExit: config.get<boolean>('forward.retryOnExit', true),
    retryDelaySeconds: config.get<number>('forward.retryDelaySeconds', 5),
    restartOnHostChange: config.get<boolean>('forward.restartOnHostChange', true),
    verifyAfterStart: config.get<boolean>('forward.verifyAfterStart', true),
    healthCheckEnabled: config.get<boolean>('forward.healthCheckEnabled', true),
    healthCheckIntervalSeconds: config.get<number>('forward.healthCheckIntervalSeconds', 10),
    healthCheckFailureThreshold: config.get<number>('forward.healthCheckFailureThreshold', 2),
    restartBackoffMaxSeconds: config.get<number>('forward.restartBackoffMaxSeconds', 30),
    singletonReuseEnabled: config.get<boolean>('singleton.reuseEnabled', true),
    singletonLeaseTtlSeconds: config.get<number>('singleton.leaseTtlSeconds', 20),
    singletonStartLockTimeoutSeconds: config.get<number>('singleton.startLockTimeoutSeconds', 15),
    applyVscodeSettings: config.get<boolean>('apply.vscodeSettings', false),
    applyRemoteMachineSettings: config.get<boolean>('apply.remoteMachineSettings', true),
    applyTerminalEnv: config.get<boolean>('apply.terminalEnv', true),
    applyGitConfig: config.get<boolean>('apply.gitConfig', true),
    applyGitGlobalConfig: config.get<boolean>('apply.gitGlobalConfig', true),
    applyGitWorkspaceConfig: config.get<boolean>('apply.gitWorkspaceConfig', true),
    applyGitForceOverride: config.get<boolean>('apply.gitForceOverride', true),
    applyServerEnvSetup: config.get<boolean>('apply.serverEnvSetup', true),
    applyRemoteStatusFile: config.get<boolean>('apply.remoteStatusFile', true),
    proxySupport: config.get<'override' | 'on' | 'off' | 'fallback'>('apply.proxySupport', 'override')
  };
}

export async function setManualProxyUrl(url: string): Promise<void> {
  await vscode.workspace.getConfiguration('remoteProxy').update('localProxy.url', url, vscode.ConfigurationTarget.Global);
  await vscode.workspace.getConfiguration('remoteProxy').update('localProxy.mode', 'manual', vscode.ConfigurationTarget.Global);
}

export async function setSshHost(host: string): Promise<void> {
  await vscode.workspace.getConfiguration('remoteProxy').update('ssh.host', host, vscode.ConfigurationTarget.Global);
}

export async function clearSshHost(): Promise<void> {
  await vscode.workspace.getConfiguration('remoteProxy').update('ssh.host', undefined, vscode.ConfigurationTarget.Global);
}

export function readHostProfile(keys: readonly string[]): HostProfile | undefined {
  const profiles = vscode.workspace.getConfiguration('remoteProxy').get<Record<string, HostProfile>>('hostProfiles', {});
  for (const key of keys) {
    const profile = profiles[key];
    if (profile && typeof profile === 'object') {
      return profile;
    }
  }
  return undefined;
}

export function applyHostProfile(config: RemoteProxyConfig, profile: HostProfile | undefined): RemoteProxyConfig {
  if (!profile) {
    return config;
  }

  return {
    ...config,
    localProxyUrl: profile.localProxyUrl ?? config.localProxyUrl,
    localProxyMode: profile.localProxyUrl ? 'manual' : config.localProxyMode,
    remotePort: profile.remotePort ?? config.remotePort,
    remoteBindHost: profile.remoteBindHost ?? config.remoteBindHost,
    noProxy: profile.noProxy ?? config.noProxy,
    applyGitConfig: profile.applyGitConfig ?? config.applyGitConfig,
    applyGitGlobalConfig: profile.applyGitGlobalConfig ?? config.applyGitGlobalConfig,
    applyGitWorkspaceConfig: profile.applyGitWorkspaceConfig ?? config.applyGitWorkspaceConfig,
    applyGitForceOverride: profile.applyGitForceOverride ?? config.applyGitForceOverride,
    applyServerEnvSetup: profile.applyServerEnvSetup ?? config.applyServerEnvSetup
  };
}
