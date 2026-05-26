import * as fs from 'fs';
import * as path from 'path';
import { DEFAULT_SSH_PROXY_EXECUTABLE, normalizeSshProxyExecutable } from './sshProxyCliUtils';

export type SshProxyExecutableSource = 'configured' | 'bundled' | 'path';

export interface SshProxyExecutableDiscovery {
  readonly executable: string;
  readonly source: SshProxyExecutableSource;
  readonly configuredValue: string;
}

export interface SshProxyExecutableDiscoveryOptions {
  readonly extensionPath?: string;
  readonly platform?: string;
  readonly arch?: string;
}

export function resolveSshProxyExecutable(
  configured: string | undefined | null,
  options: SshProxyExecutableDiscoveryOptions = {},
): SshProxyExecutableDiscovery {
  return resolveSshProxyExecutableCandidates(configured, options)[0];
}

export function resolveSshProxyExecutableCandidates(
  configured: string | undefined | null,
  options: SshProxyExecutableDiscoveryOptions = {},
): SshProxyExecutableDiscovery[] {
  const configuredValue = (configured ?? '').trim();
  const executable = normalizeSshProxyExecutable(configuredValue);
  if (!configuredValue) {
    return [{
      executable,
      source: 'path',
      configuredValue,
    }];
  }

  if (configuredValue && executable !== DEFAULT_SSH_PROXY_EXECUTABLE) {
    return [{
      executable,
      source: 'configured',
      configuredValue,
    }];
  }

  const bundled = resolveBundledSshProxyExecutable(options);
  if (bundled) {
    return [
      {
        executable: bundled,
        source: 'bundled',
        configuredValue,
      },
      {
        executable,
        source: 'path',
        configuredValue,
      },
    ];
  }

  return [{
    executable,
    source: 'path',
    configuredValue,
  }];
}

export function bundledSshProxyRelativePath(options: SshProxyExecutableDiscoveryOptions = {}): string | undefined {
  const platform = options.platform ?? process.platform;
  const arch = options.arch ?? process.arch;
  if (platform === 'win32' && arch === 'x64') {
    return path.join('assets', 'bin', 'win32-x64', 'ssh_proxy.exe');
  }
  if (platform === 'linux' && arch === 'x64') {
    return path.join('assets', 'bin', 'linux-x64', 'ssh_proxy');
  }
  return undefined;
}

export function resolveBundledSshProxyExecutable(options: SshProxyExecutableDiscoveryOptions = {}): string | undefined {
  const relative = bundledSshProxyRelativePath(options);
  if (!relative) {
    return undefined;
  }
  const candidate = path.join(options.extensionPath ?? defaultExtensionPath(), relative);
  if (!isFile(candidate)) {
    return undefined;
  }
  ensureExecutableBit(candidate, options.platform ?? process.platform);
  return candidate;
}

export function sshProxyUnavailableCandidatesMessage(discoveries: readonly SshProxyExecutableDiscovery[]): string {
  if (discoveries.length === 0) {
    return 'ssh_proxy was not available. Set remoteProxy.sshProxy.executable or install the ssh_proxy release binary.';
  }
  if (discoveries.length === 1) {
    return sshProxyUnavailableMessage(discoveries[0]);
  }
  const tried = discoveries.map((discovery) => describeSshProxyDiscovery(discovery)).join('; ');
  return `ssh_proxy was not available from the bundled extension binary or PATH. Tried: ${tried}. Set remoteProxy.sshProxy.executable or install the ssh_proxy release binary.`;
}

function defaultExtensionPath(): string {
  return path.resolve(__dirname, '..');
}

function isFile(candidate: string): boolean {
  try {
    return fs.statSync(candidate).isFile();
  } catch {
    return false;
  }
}

function ensureExecutableBit(candidate: string, platform: string): void {
  if (platform === 'win32') {
    return;
  }
  try {
    fs.chmodSync(candidate, 0o755);
  } catch {
    // A read-only extension install can still report a useful spawn error later.
  }
}

export function sshProxyUnavailableMessage(discovery: SshProxyExecutableDiscovery): string {
  if (discovery.source === 'bundled') {
    return `ssh_proxy was not available from the bundled extension binary at "${discovery.executable}". Set remoteProxy.sshProxy.executable or install the ssh_proxy release binary.`;
  }
  if (discovery.source === 'path') {
    return 'ssh_proxy was not found on PATH. Set remoteProxy.sshProxy.executable or install the ssh_proxy release binary.';
  }
  return `ssh_proxy was not available at "${discovery.executable}". Check remoteProxy.sshProxy.executable or install the ssh_proxy release binary.`;
}

export function describeSshProxyDiscovery(discovery: SshProxyExecutableDiscovery): string {
  if (discovery.source === 'bundled') {
    return `bundled ssh_proxy (${discovery.executable})`;
  }
  if (discovery.source === 'path') {
    return `ssh_proxy from PATH (${discovery.executable})`;
  }
  return `ssh_proxy from remoteProxy.sshProxy.executable (${discovery.executable})`;
}
