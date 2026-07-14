const assert = require('node:assert/strict');
const test = require('node:test');

const {
  buildSshProxyDaemonInstallArgs,
  buildSshProxyDownArgs,
  buildSshProxyVscodeApplySettingsArgs,
  buildSshProxyVscodeStatusArgs,
  buildSshProxyVscodeUpArgs,
  formatSshProxyCommand,
  isSshProxyDaemonInstallCancelledMessage,
  normalizeSshProxyExecutable,
  parseSshProxyJson,
  redactSshProxyArgs,
  redactSshProxyText,
} = require('../out/sshProxyCliUtils');

test('normalizes an empty ssh_proxy executable to the PATH command', () => {
  assert.equal(normalizeSshProxyExecutable(''), 'ssh_proxy');
  assert.equal(normalizeSshProxyExecutable('  "C:/Tools/ssh_proxy.exe"  '), 'C:/Tools/ssh_proxy.exe');
});

test('builds JSON command shapes consumed by the extension', () => {
  assert.deepEqual(
    buildSshProxyVscodeUpArgs({
      target: '126',
      workspace: 'window-a',
      localProxy: 'http://127.0.0.1:10808/',
      remoteBind: '127.0.0.1',
      remotePort: 17890,
      connectMode: 'reverse-link',
    }),
    [
      'vscode',
      'up',
      '--target',
      '126',
      '--workspace',
      'window-a',
      '--local-proxy',
      'http://127.0.0.1:10808/',
      '--remote-bind',
      '127.0.0.1',
      '--remote-port',
      '17890',
      '--connect-mode',
      'reverse-link',
      '--json',
    ],
  );
  assert.deepEqual(
    buildSshProxyVscodeStatusArgs({ workspace: 'window-a', target: '126' }),
    ['vscode', 'status', '--workspace', 'window-a', '--target', '126', '--json'],
  );
  assert.deepEqual(
    buildSshProxyVscodeUpArgs({
      target: '126',
      workspace: 'window-a',
      localProxy: 'http://127.0.0.1:10808/',
      remoteBind: '127.0.0.1',
      remotePort: 17890,
      remoteAutoPickPort: false,
      remotePortRangeSize: 3,
      connectMode: 'reverse-link',
    }),
    [
      'vscode',
      'up',
      '--target',
      '126',
      '--workspace',
      'window-a',
      '--local-proxy',
      'http://127.0.0.1:10808/',
      '--remote-bind',
      '127.0.0.1',
      '--remote-port',
      '17890',
      '--no-remote-auto-pick',
      '--remote-port-range-size',
      '3',
      '--connect-mode',
      'reverse-link',
      '--json',
    ],
  );
  assert.deepEqual(
    buildSshProxyVscodeUpArgs({
      target: '102',
      workspace: 'user@10.10.100.71',
      localProxy: 'http://127.0.0.1:10808/',
      remoteBind: '127.0.0.1',
      remotePort: 17890,
      connectMode: 'reverse-link',
      sshTarget: {
        hostName: '10.10.100.71',
        user: 'wenhongli',
        port: 10022,
        identityFiles: ['C:/Users/whl/.ssh/id_rsa'],
        configFile: 'C:/Users/whl/.ssh/config',
        knownHostsFile: 'C:/Users/whl/.ssh/known_hosts',
        proxyJump: ['hub'],
        acceptNew: true,
      },
    }),
    [
      'vscode',
      'up',
      '--target',
      '102',
      '--workspace',
      'user@10.10.100.71',
      '--local-proxy',
      'http://127.0.0.1:10808/',
      '--remote-bind',
      '127.0.0.1',
      '--remote-port',
      '17890',
      '--connect-mode',
      'reverse-link',
      '--ssh-host-name',
      '10.10.100.71',
      '--ssh-user',
      'wenhongli',
      '--ssh-port',
      '10022',
      '--ssh-identity',
      'C:/Users/whl/.ssh/id_rsa',
      '--ssh-config',
      'C:/Users/whl/.ssh/config',
      '--ssh-known-hosts',
      'C:/Users/whl/.ssh/known_hosts',
      '--ssh-jump',
      'hub',
      '--ssh-accept-new',
      '--json',
    ],
  );
  assert.deepEqual(
    buildSshProxyVscodeApplySettingsArgs({
      target: '126',
      workspace: 'window-a',
      proxyUrl: 'http://127.0.0.1:17890/',
    }),
    [
      'vscode',
      'apply-settings',
      '--target',
      '126',
      '--workspace',
      'window-a',
      '--proxy-url',
      'http://127.0.0.1:17890/',
      '--json',
    ],
  );
  assert.deepEqual(
    buildSshProxyDownArgs({ routeId: 'v3-window-a', workspace: 'window-a', target: '126' }),
    ['down', '--route-id', 'v3-window-a', '--workspace', 'window-a', '--target', '126', '--json'],
  );
  assert.deepEqual(
    buildSshProxyDaemonInstallArgs({ scope: 'system', elevate: true }),
    ['daemon', 'install', '--scope', 'system', '--elevate'],
  );
});

test('redacts token flags and proxy URL credentials', () => {
  assert.deepEqual(
    redactSshProxyArgs([
      'route',
      'edge',
      '--token',
      'secret-token',
      '--remote-token=sample-remote-token',
      '--egress-proxy',
      'http://demo-user:demo-credential@127.0.0.1:18080',
    ]),
    [
      'route',
      'edge',
      '--token',
      '<redacted>',
      '--remote-token=<redacted>',
      '--egress-proxy',
      'http://<redacted>@127.0.0.1:18080',
    ],
  );
});

test('classifies cancelled elevated daemon installs', () => {
  assert.equal(
    isSshProxyDaemonInstallCancelledMessage('powershell.exe exited with status exit code: 0xc000013a'),
    true,
  );
  assert.equal(
    isSshProxyDaemonInstallCancelledMessage('ssh_proxy daemon install failed with code 1223'),
    true,
  );
  assert.equal(
    isSshProxyDaemonInstallCancelledMessage('ssh_proxy daemon install cancelled_by_user'),
    true,
  );
  assert.equal(
    isSshProxyDaemonInstallCancelledMessage('failed to copy binary because it is in use'),
    false,
  );
});

test('redacts credentials in free-form text', () => {
  assert.equal(
    redactSshProxyText('failed to use socks5://demo-user:demo-credential@example.test:1080/path'),
    'failed to use socks5://<redacted>@example.test:1080/path',
  );
});

test('formats redacted command lines for logs', () => {
  assert.equal(
    formatSshProxyCommand('C:/Program Files/ssh_proxy.exe', ['route', '--token', 'secret']),
    '"C:/Program Files/ssh_proxy.exe" route --token <redacted>',
  );
});

test('parses ssh_proxy JSON output and reports invalid JSON with context', () => {
  assert.deepEqual(parseSshProxyJson('{"ok":true}\n'), { ok: true });
  assert.equal(parseSshProxyJson(''), null);
  assert.throws(
    () => parseSshProxyJson('not json with http://demo-user:demo-credential@example.test', 'route explain'),
    /route explain did not return valid JSON:.*http:\/\/<redacted>@example\.test/s,
  );
});
