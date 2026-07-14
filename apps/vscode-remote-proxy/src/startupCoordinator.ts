import { RemoteContext, RemoteProxyConfig } from './types';

export type AutoStartDecision =
  | {
    readonly action: 'start';
  }
  | {
    readonly action: 'skip';
    readonly outputLine: string;
    readonly statusText?: string;
  };

export type StartPreflightDecision =
  | {
    readonly action: 'continue';
  }
  | {
    readonly action: 'disabled';
    readonly informationMessage: string;
  }
  | {
    readonly action: 'unsupported-remote';
    readonly warningMessage: string;
    readonly outputLine: string;
  };

export type RemoteActionPreflightDecision =
  | {
    readonly action: 'continue';
  }
  | {
    readonly action: 'unsupported-remote';
    readonly warningMessage: string;
  };

export function planAutoStart(config: RemoteProxyConfig, remote: RemoteContext): AutoStartDecision {
  if (!config.enabled || !config.autoStart) {
    return {
      action: 'skip',
      outputLine: 'Remote Proxy is disabled or autoStart is off.',
    };
  }

  if (remote.kind === 'none') {
    return {
      action: 'skip',
      outputLine: 'Not in a remote window; auto-start skipped.',
    };
  }

  if (remote.kind !== 'ssh') {
    return {
      action: 'skip',
      outputLine: `Remote kind "${remote.name ?? remote.kind}" detected; only SSH is currently auto-started.`,
      statusText: `$(circle-slash) Proxy ${remote.kind}`,
    };
  }

  return { action: 'start' };
}

export function checkStartPreflight(config: RemoteProxyConfig, remote: RemoteContext): StartPreflightDecision {
  if (!config.enabled) {
    return {
      action: 'disabled',
      informationMessage: 'Remote Proxy is disabled.',
    };
  }

  if (remote.kind !== 'ssh' && !config.sshHostOverride.trim()) {
    return {
      action: 'unsupported-remote',
      warningMessage: 'Remote Proxy currently supports automatic forwarding for Remote SSH windows.',
      outputLine: `Cannot start: remote=${JSON.stringify(remote)}`,
    };
  }

  return { action: 'continue' };
}

export function checkCleanupPreflight(config: RemoteProxyConfig, remote: RemoteContext): RemoteActionPreflightDecision {
  if (remote.kind !== 'ssh' && !config.sshHostOverride.trim()) {
    return {
      action: 'unsupported-remote',
      warningMessage: 'Remote Proxy cleanup requires a Remote SSH window or remoteProxy.ssh.host.',
    };
  }

  return { action: 'continue' };
}

export function checkApplySettingsPreflight(config: RemoteProxyConfig, remote: RemoteContext): RemoteActionPreflightDecision {
  if (remote.kind !== 'ssh' && !config.sshHostOverride.trim()) {
    return {
      action: 'unsupported-remote',
      warningMessage: 'Remote Proxy settings currently require a Remote SSH window or remoteProxy.ssh.host.',
    };
  }

  return { action: 'continue' };
}
