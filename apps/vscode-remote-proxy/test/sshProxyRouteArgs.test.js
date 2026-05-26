const assert = require('node:assert/strict');
const test = require('node:test');

const { buildSshProxyRouteArgs, hashSshProxyRouteTarget } = require('../out/sshProxyRouteArgs');

function config(overrides = {}) {
  return {
    sshArgs: [],
    sshProxyRouteVolatile: true,
    ...overrides,
  };
}

function proxy(overrides = {}) {
  return {
    local: {
      url: 'http://127.0.0.1:18080',
      scheme: 'http',
      host: '127.0.0.1',
      port: 18080,
      source: 'test',
    },
    remoteUrl: 'http://127.0.0.1:17890',
    remotePort: 17890,
    remoteBindHost: '127.0.0.1',
    ...overrides,
  };
}

test('builds stable reverse-link route template', () => {
  assert.deepEqual(
    buildSshProxyRouteArgs(config(), 'edge', proxy()),
    [
      'edge',
      '--direction',
      'remote-uses-local',
      '--connect-mode',
      'reverse-link',
      '--bind',
      '127.0.0.1',
      '--port',
      '17890',
      '--egress-proxy',
      'http://127.0.0.1:18080',
      '--id',
      `vscode-remote-proxy-${hashSshProxyRouteTarget('edge')}`,
      '--volatile',
    ],
  );
});

test('keeps user ssh args before route options', () => {
  assert.deepEqual(
    buildSshProxyRouteArgs(
      config({ sshArgs: ['-F', 'C:/Users/me/.ssh/config'] }),
      'edge',
      proxy({ routeId: 'vscode-remote-proxy-custom' }),
    ),
    [
      'edge',
      '--ssh-arg',
      '-F',
      '--ssh-arg',
      'C:/Users/me/.ssh/config',
      '--direction',
      'remote-uses-local',
      '--connect-mode',
      'reverse-link',
      '--bind',
      '127.0.0.1',
      '--port',
      '17890',
      '--egress-proxy',
      'http://127.0.0.1:18080',
      '--id',
      'vscode-remote-proxy-custom',
      '--volatile',
    ],
  );
});

test('targets a session daemon endpoint when the service fallback is active', () => {
  assert.deepEqual(
    buildSshProxyRouteArgs(
      config(),
      'edge',
      proxy({ routeId: 'vscode-remote-proxy-custom' }),
      {
        endpoint: 'tcp://127.0.0.1:19181',
        token: 'session-token',
      },
    ),
    [
      'edge',
      '--direction',
      'remote-uses-local',
      '--connect-mode',
      'reverse-link',
      '--bind',
      '127.0.0.1',
      '--port',
      '17890',
      '--egress-proxy',
      'http://127.0.0.1:18080',
      '--id',
      'vscode-remote-proxy-custom',
      '--endpoint',
      'tcp://127.0.0.1:19181',
      '--token',
      'session-token',
      '--volatile',
    ],
  );
});

test('omits volatile flag when persistent routes are explicitly requested', () => {
  assert.equal(
    buildSshProxyRouteArgs(
      config({ sshProxyRouteVolatile: false }),
      'edge',
      proxy(),
    ).includes('--volatile'),
    false,
  );
});
