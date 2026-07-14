import * as vscode from 'vscode';
import { parseSshHostAuthority } from './sshAuthority';
import { RemoteContext, SupportedRemoteKind } from './types';

export function getRemoteContext(sshHostOverride: string): RemoteContext {
  const name = vscode.env.remoteName;
  const authority = findRemoteAuthority();
  const kind = getRemoteKind(name, authority);
  const sshHost = sshHostOverride.trim() || (kind === 'ssh' ? parseSshHost(authority) : undefined);

  return {
    kind,
    name,
    authority,
    sshHost,
    sshHostSource: sshHost ? (sshHostOverride.trim() ? 'remoteProxy.ssh.host' : 'vscode-remote URI authority') : undefined
  };
}

function getRemoteKind(name: string | undefined, authority: string | undefined): SupportedRemoteKind {
  if (!name && !authority) {
    return 'none';
  }

  if (name === 'ssh-remote' || authority?.startsWith('ssh-remote+')) {
    return 'ssh';
  }

  if (name === 'wsl' || authority?.startsWith('wsl+')) {
    return 'wsl';
  }

  if (name === 'dev-container' || authority?.startsWith('dev-container+')) {
    return 'dev-container';
  }

  return 'other';
}

function findRemoteAuthority(): string | undefined {
  for (const folder of vscode.workspace.workspaceFolders ?? []) {
    if (folder.uri.scheme === 'vscode-remote' && folder.uri.authority) {
      return folder.uri.authority;
    }
  }

  const active = vscode.window.activeTextEditor?.document.uri;
  if (active?.scheme === 'vscode-remote' && active.authority) {
    return active.authority;
  }

  for (const document of vscode.workspace.textDocuments) {
    if (document.uri.scheme === 'vscode-remote' && document.uri.authority) {
      return document.uri.authority;
    }
  }

  return undefined;
}

function parseSshHost(authority: string | undefined): string | undefined {
  if (!authority) {
    return undefined;
  }
  return parseSshHostAuthority(authority);
}
