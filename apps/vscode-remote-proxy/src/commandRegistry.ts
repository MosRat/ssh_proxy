import * as vscode from 'vscode';
import { REMOTE_PROXY_COMMANDS, RemoteProxyCommandHandler } from './commandDefinitions';

export type RemoteProxyCommandController = Record<RemoteProxyCommandHandler, () => unknown> & {
  onConfigChanged(): unknown;
};

export function registerRemoteProxyCommands(
  context: vscode.ExtensionContext,
  controllerProvider: () => RemoteProxyCommandController | undefined,
): void {
  context.subscriptions.push(
    ...REMOTE_PROXY_COMMANDS.map(({ command, handler }) => (
      vscode.commands.registerCommand(command, () => controllerProvider()?.[handler]())
    )),
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration('remoteProxy')) {
        controllerProvider()?.onConfigChanged();
      }
    }),
  );
}
