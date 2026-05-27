import type { AppliedProxy } from './types';

export type RemoteProxyMenuStatus = 'stopped' | 'starting' | 'running' | 'failed';

export interface RemoteProxyMenuSnapshot {
  readonly proxy: AppliedProxy | undefined;
}

export interface RemoteProxyMenuActions {
  readonly start: () => unknown;
  readonly restart: () => unknown;
  readonly stop: () => unknown;
  readonly diagnose: () => unknown;
  readonly applySettingsOnly: () => unknown;
  readonly cleanupRemote: () => unknown;
  readonly pickLocalProxy: () => unknown;
  readonly pickSshHost: () => unknown;
  readonly clearSshHost: () => unknown;
  readonly openSettings: () => unknown;
  readonly showOutput: () => unknown;
}

export interface RemoteProxyMenuItem {
  readonly label: string;
  readonly description: string;
  readonly run: () => unknown;
}

export function buildRemoteProxyMenuItems(
  status: RemoteProxyMenuStatus,
  snapshot: RemoteProxyMenuSnapshot,
  actions: RemoteProxyMenuActions,
): RemoteProxyMenuItem[] {
  return [
    {
      label: status === 'running' ? '$(debug-restart) Restart' : '$(play) Start',
      description: status === 'running' ? 'Rebuild the SSH reverse tunnel' : 'Start proxy forwarding',
      run: () => status === 'running' ? actions.restart() : actions.start(),
    },
    {
      label: '$(debug-stop) Stop',
      description: status === 'running' ? 'Stop the current SSH reverse tunnel' : 'Forwarder is not running',
      run: () => actions.stop(),
    },
    {
      label: '$(pulse) Diagnose',
      description: 'Print status and verify the remote forwarded port',
      run: () => actions.diagnose(),
    },
    {
      label: '$(gear) Apply Remote Settings',
      description: snapshot.proxy ? `Write ${snapshot.proxy.remoteUrl} to remote VS Code, terminal, and Git settings` : 'Apply settings from the configured remote port',
      run: () => actions.applySettingsOnly(),
    },
    {
      label: '$(trash) Clean Remote Settings',
      description: 'Remove managed remote proxy settings, terminal env, server-env block, status file, and Git proxy',
      run: () => actions.cleanupRemote(),
    },
    {
      label: '$(plug) Pick Local Proxy',
      description: 'Select or enter the local proxy URL',
      run: () => actions.pickLocalProxy(),
    },
    {
      label: '$(server) Pick SSH Host',
      description: 'Set an explicit SSH host override',
      run: () => actions.pickSshHost(),
    },
    {
      label: '$(close) Clear SSH Host Override',
      description: 'Return to automatic Remote SSH host detection',
      run: () => actions.clearSshHost(),
    },
    {
      label: '$(settings-gear) Open Settings',
      description: 'Open Remote Proxy settings',
      run: () => actions.openSettings(),
    },
    {
      label: '$(output) Show Output',
      description: 'Open the Remote Proxy output channel',
      run: () => actions.showOutput(),
    },
  ];
}
