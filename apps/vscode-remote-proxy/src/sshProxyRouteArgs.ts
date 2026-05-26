import { AppliedProxy, RemoteProxyConfig } from './types';

export function buildSshProxyRouteArgs(config: RemoteProxyConfig, sshHost: string, proxy: AppliedProxy): string[] {
  const args = [
    sshHost,
    '--direction',
    'remote-uses-local',
    '--connect-mode',
    'reverse-link',
    '--bind',
    proxy.remoteBindHost,
    '--port',
    String(proxy.remotePort),
    '--egress-proxy',
    proxy.local.url,
    '--id',
    proxy.routeId ?? `vscode-remote-proxy-${hashSshProxyRouteTarget(sshHost)}`,
  ];

  if (config.sshProxyRouteVolatile) {
    args.push('--volatile');
  }

  for (let index = config.sshArgs.length - 1; index >= 0; index -= 1) {
    args.splice(1, 0, '--ssh-arg', config.sshArgs[index]);
  }

  return args;
}

export function hashSshProxyRouteTarget(value: string): string {
  let hash = 0;
  for (let index = 0; index < value.length; index += 1) {
    hash = (hash * 33 + value.charCodeAt(index)) >>> 0;
  }
  return hash.toString(16);
}
