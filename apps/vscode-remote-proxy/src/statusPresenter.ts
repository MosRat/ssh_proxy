import * as vscode from 'vscode';
import { ForwardingBackendStatus } from './forwardingBackend';
import { describeSshProxyDaemonHealth, describeSshProxyRouteHealth } from './statusDisplay';
import { SshProxyKernelStatusSnapshot } from './sshProxyKernelStatus';
import { AppliedProxy, ForwardingBackendKind } from './types';

export interface RemoteProxyStatusPresentation {
  readonly status: ForwardingBackendStatus;
  readonly effectiveStatus: string;
  readonly backend: ForwardingBackendKind;
  readonly sshHost: string | undefined;
  readonly proxy: AppliedProxy | undefined;
  readonly kernelStatus: SshProxyKernelStatusSnapshot | undefined;
  readonly lastError: string | undefined;
}

export interface RemoteProxyMenuSnapshot {
  readonly detectedHost: string | undefined;
  readonly proxy: AppliedProxy | undefined;
}

export function updateRemoteProxyStatusBar(
  statusBar: vscode.StatusBarItem,
  input: RemoteProxyStatusPresentation,
  text?: string,
): void {
  if (text) {
    statusBar.text = text;
    return;
  }

  switch (input.status) {
    case 'running':
      statusBar.text = `$(radio-tower) Proxy ${input.sshHost ?? ''}${input.proxy ? `:${input.proxy.remotePort}` : ''}`.trim();
      statusBar.backgroundColor = undefined;
      break;
    case 'starting':
      statusBar.text = '$(sync~spin) Proxy';
      statusBar.backgroundColor = undefined;
      break;
    case 'failed':
      statusBar.text = '$(warning) Proxy';
      statusBar.backgroundColor = new vscode.ThemeColor('statusBarItem.warningBackground');
      break;
    default:
      statusBar.text = '$(circle-large-outline) Proxy';
      statusBar.backgroundColor = undefined;
      break;
  }
  statusBar.tooltip = buildRemoteProxyStatusTooltip(input);
}

export function buildRemoteProxyStatusTooltip(input: RemoteProxyStatusPresentation): vscode.MarkdownString {
  const proxy = input.proxy;
  const markdown = new vscode.MarkdownString(undefined, true);
  markdown.isTrusted = true;
  markdown.appendMarkdown('**Remote Proxy**\n\n');
  markdown.appendMarkdown(`Status: \`${input.effectiveStatus}\`\n\n`);
  markdown.appendMarkdown(`Backend: \`${input.backend}\`\n\n`);
  markdown.appendMarkdown(`SSH host: \`${input.sshHost ?? 'not active'}\`\n\n`);
  markdown.appendMarkdown(`Remote proxy: \`${proxy?.remoteUrl ?? 'not active'}\`\n\n`);
  markdown.appendMarkdown(`Route: \`${proxy?.routeId ?? 'not active'}\`\n\n`);
  markdown.appendMarkdown(`Transport: \`${proxy?.selectedTransport ?? 'not active'}\`\n\n`);
  markdown.appendMarkdown(`Fallback: \`${proxy?.fallbackReason ?? 'none'}\`\n\n`);
  markdown.appendMarkdown(`Daemon health: \`${sanitizeMarkdownValue(describeSshProxyDaemonHealth(input.backend, input.kernelStatus))}\`\n\n`);
  markdown.appendMarkdown(`Route health: \`${sanitizeMarkdownValue(describeSshProxyRouteHealth(input.backend, input.kernelStatus))}\`\n\n`);
  markdown.appendMarkdown(`Local proxy: \`${proxy?.local.url ?? 'not active'}\`\n\n`);
  if (input.lastError) {
    markdown.appendMarkdown(`Last error: \`${sanitizeMarkdownValue(input.lastError)}\`\n\n`);
  }
  markdown.appendMarkdown('[Open Menu](command:remoteProxy.openMenu) | [Diagnose](command:remoteProxy.diagnose) | [Settings](command:remoteProxy.openSettings)');
  return markdown;
}

export function describeRemoteProxyMenuPlaceholder(
  snapshot: RemoteProxyMenuSnapshot,
  status: ForwardingBackendStatus,
): string {
  const proxy = snapshot.proxy?.remoteUrl ?? 'not active';
  const host = snapshot.detectedHost ?? 'host unresolved';
  const transport = snapshot.proxy?.selectedTransport ?? 'transport unknown';
  return `${status} | ${host} | ${transport} | ${proxy}`;
}

function sanitizeMarkdownValue(value: string): string {
  return value.replace(/`/g, "'");
}
