const assert = require('node:assert/strict');
const test = require('node:test');

const {
  buildSshProxyNodeControlShutdownArgs,
  buildSshProxyNodeControlStatusArgs,
  buildSshProxyNodeDaemonArgs,
  buildSshProxyDownArgs,
  buildSshProxyServiceEnsureArgs,
  buildSshProxyServiceInstallArgs,
  buildSshProxyServiceStatusArgs,
  buildSshProxyRoutesArgs,
  buildSshProxyStopRouteArgs,
  buildSshProxyVscodeStatusArgs,
  buildSshProxyVscodeUpArgs,
  formatSshProxyCommand,
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
  assert.deepEqual(buildSshProxyServiceStatusArgs(), ['service', '--json', 'status']);
  assert.deepEqual(buildSshProxyServiceEnsureArgs(), ['service', '--scope', 'auto', '--json', 'ensure']);
  assert.deepEqual(buildSshProxyServiceEnsureArgs('system', { elevate: true }), ['service', '--scope', 'system', '--json', '--elevate', 'ensure']);
  assert.deepEqual(buildSshProxyServiceInstallArgs(), ['service', '--scope', 'auto', 'install']);
  assert.deepEqual(buildSshProxyServiceInstallArgs('user'), ['service', '--scope', 'user', 'install']);
  assert.deepEqual(buildSshProxyRoutesArgs(), ['node', 'control', '--json', 'routes']);
  assert.deepEqual(buildSshProxyStopRouteArgs('route-1'), ['node', 'control', '--json', 'stop-route', 'route-1']);
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
    buildSshProxyDownArgs({ routeId: 'v3-window-a', workspace: 'window-a', target: '126' }),
    ['down', '--route-id', 'v3-window-a', '--workspace', 'window-a', '--target', '126', '--json'],
  );
});

test('builds node control commands for a session daemon endpoint', () => {
  const connection = {
    endpoint: 'tcp://127.0.0.1:19181',
    token: 'session-token',
  };
  assert.deepEqual(
    buildSshProxyNodeControlStatusArgs(connection),
    ['node', 'control', '--endpoint', 'tcp://127.0.0.1:19181', '--token', 'session-token', '--json', 'status'],
  );
  assert.deepEqual(
    buildSshProxyRoutesArgs(connection),
    ['node', 'control', '--endpoint', 'tcp://127.0.0.1:19181', '--token', 'session-token', '--json', 'routes'],
  );
  assert.deepEqual(
    buildSshProxyStopRouteArgs('route-1', connection),
    ['node', 'control', '--endpoint', 'tcp://127.0.0.1:19181', '--token', 'session-token', '--json', 'stop-route', 'route-1'],
  );
  assert.deepEqual(
    buildSshProxyNodeControlShutdownArgs(connection),
    ['node', 'control', '--endpoint', 'tcp://127.0.0.1:19181', '--token', 'session-token', '--json', 'shutdown'],
  );
});

test('builds session daemon command shape', () => {
  assert.deepEqual(
    buildSshProxyNodeDaemonArgs({
      control: 'tcp://127.0.0.1:19181',
      transport: '127.0.0.1:19180',
      token: 'session-token',
      name: 'vscode-remote-proxy-session',
    }),
    [
      'node',
      'daemon',
      '--control',
      'tcp://127.0.0.1:19181',
      '--transport',
      '127.0.0.1:19180',
      '--token',
      'session-token',
      '--no-route-autostart',
      '--name',
      'vscode-remote-proxy-session',
    ],
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
