export const REMOTE_PROXY_COMMANDS = [
  { command: 'remoteProxy.start', handler: 'start' },
  { command: 'remoteProxy.stop', handler: 'stop' },
  { command: 'remoteProxy.restart', handler: 'restart' },
  { command: 'remoteProxy.applySettings', handler: 'applySettingsOnly' },
  { command: 'remoteProxy.cleanupRemote', handler: 'cleanupRemote' },
  { command: 'remoteProxy.pickLocalProxy', handler: 'pickLocalProxy' },
  { command: 'remoteProxy.pickSshHost', handler: 'pickSshHost' },
  { command: 'remoteProxy.clearSshHost', handler: 'clearSshHost' },
  { command: 'remoteProxy.openMenu', handler: 'openMenu' },
  { command: 'remoteProxy.diagnose', handler: 'diagnose' },
  { command: 'remoteProxy.showOutput', handler: 'showOutput' },
  { command: 'remoteProxy.openSettings', handler: 'openSettings' },
  { command: 'remoteProxy.showStatus', handler: 'showStatus' },
] as const;

export type RemoteProxyCommandHandler = typeof REMOTE_PROXY_COMMANDS[number]['handler'];
