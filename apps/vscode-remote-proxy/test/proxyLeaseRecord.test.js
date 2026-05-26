const assert = require('node:assert/strict');
const test = require('node:test');

const {
  buildProxyLeaseState,
  normalizeProxyLeaseState,
} = require('../out/proxyLeaseRecord');

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

test('builds v2 leases with backend and route metadata', () => {
  const state = buildProxyLeaseState({
    ownerId: 'owner-1',
    pid: 42,
    targetKey: 'edge',
    sshHost: 'edge',
    proxy: proxy({
      backend: 'ssh_proxy',
      routeId: 'vscode-remote-proxy-edge',
      routeOwner: 'local',
      selectedTransport: 'ssh-reverse-link',
      connectMode: 'reverse-link',
      fallbackReason: 'ssh-only topology',
      cleanupCommand: 'ssh_proxy node control stop-route vscode-remote-proxy-edge',
    }),
    now: 1000,
  });

  assert.equal(state.version, 2);
  assert.equal(state.backend, 'ssh_proxy');
  assert.equal(state.routeId, 'vscode-remote-proxy-edge');
  assert.equal(state.routeOwner, 'local');
  assert.equal(state.selectedTransport, 'ssh-reverse-link');
  assert.equal(state.remoteUrl, 'http://127.0.0.1:17890');
  assert.equal(state.proxy.backend, 'ssh_proxy');
  assert.equal(state.proxy.routeOwner, 'local');
  assert.equal(state.startedAt, 1000);
  assert.equal(state.updatedAt, 1000);
});

test('preserves startedAt for same-owner lease refreshes', () => {
  const previous = buildProxyLeaseState({
    ownerId: 'owner-1',
    pid: 42,
    targetKey: 'edge',
    sshHost: 'edge',
    proxy: proxy({ backend: 'openssh' }),
    now: 1000,
  });
  const refreshed = buildProxyLeaseState({
    ownerId: 'owner-1',
    pid: 43,
    targetKey: 'edge',
    sshHost: 'edge',
    proxy: proxy({ backend: 'openssh' }),
    previous,
    now: 2000,
  });

  assert.equal(refreshed.startedAt, 1000);
  assert.equal(refreshed.updatedAt, 2000);
});

test('normalizes old v1 leases as OpenSSH-compatible leases', () => {
  const state = normalizeProxyLeaseState({
    version: 1,
    ownerId: 'legacy-owner',
    pid: 77,
    targetKey: 'edge',
    sshHost: 'edge',
    startedAt: 1000,
    updatedAt: 1500,
    proxy: proxy(),
  });

  assert.ok(state);
  assert.equal(state.version, 1);
  assert.equal(state.backend, 'openssh');
  assert.equal(state.routeOwner, 'openssh');
  assert.equal(state.selectedTransport, 'openssh-reverse');
  assert.equal(state.remoteUrl, 'http://127.0.0.1:17890');
  assert.equal(state.proxy.backend, 'openssh');
});

test('normalizes v2 leases that omitted duplicated top-level metadata', () => {
  const state = normalizeProxyLeaseState({
    version: 2,
    ownerId: 'owner-1',
    pid: 42,
    targetKey: 'edge',
    sshHost: 'edge',
    startedAt: 1000,
    updatedAt: 1500,
    backend: 'ssh_proxy',
    proxy: proxy({
      routeId: 'route-1',
      selectedTransport: 'tls-tcp',
    }),
  });

  assert.ok(state);
  assert.equal(state.backend, 'ssh_proxy');
  assert.equal(state.routeId, 'route-1');
  assert.equal(state.routeOwner, 'ssh_proxy');
  assert.equal(state.selectedTransport, 'tls-tcp');
  assert.equal(state.remoteUrl, 'http://127.0.0.1:17890');
  assert.equal(state.proxy.backend, 'ssh_proxy');
});
