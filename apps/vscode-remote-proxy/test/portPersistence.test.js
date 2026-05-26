const assert = require('node:assert/strict');
const test = require('node:test');

const {
  appliedProxyFromRemoteStatus,
  buildRemotePortCandidates,
  remoteStatusMatchesCurrentProxy,
} = require('../out/portPersistence');

function config(overrides = {}) {
  return {
    remoteAutoPickPort: true,
    remotePort: 17890,
    remotePortRangeSize: 4,
    remoteBindHost: '127.0.0.1',
    ...overrides,
  };
}

function local(overrides = {}) {
  return {
    url: 'http://127.0.0.1:18080',
    scheme: 'http',
    host: '127.0.0.1',
    port: 18080,
    source: 'test',
    ...overrides,
  };
}

function proxy(port, overrides = {}) {
  return {
    local: local(),
    remoteUrl: `http://127.0.0.1:${port}`,
    remotePort: port,
    remoteBindHost: '127.0.0.1',
    ...overrides,
  };
}

test('orders sticky ports before configured range', () => {
  const ports = buildRemotePortCandidates({
    config: config(),
    local: local(),
    currentProxy: proxy(17900),
    preferredPort: 17901,
    preferredBindHost: '127.0.0.1',
    lease: { proxy: proxy(17902) },
    remoteStatus: {
      port: 17903,
      bindHost: '127.0.0.1',
      localProxyUrl: 'http://127.0.0.1:18080',
    },
  });

  assert.deepEqual(ports, [17900, 17901, 17902, 17903, 17890, 17891, 17892, 17893]);
});

test('deduplicates sticky candidates and ignores mismatched bind/local proxy', () => {
  const ports = buildRemotePortCandidates({
    config: config(),
    local: local(),
    currentProxy: proxy(17891),
    preferredPort: 17891,
    preferredBindHost: '127.0.0.1',
    lease: { proxy: proxy(17890, { remoteBindHost: '0.0.0.0' }) },
    remoteStatus: {
      port: 17903,
      bindHost: '127.0.0.1',
      localProxyUrl: 'http://127.0.0.1:7890',
    },
  });

  assert.deepEqual(ports, [17891, 17890, 17892, 17893]);
});

test('converts matching remote status into an adopted proxy', () => {
  const status = {
    proxyUrl: 'http://127.0.0.1:17903',
    bindHost: '127.0.0.1',
    port: 17903,
    localProxyUrl: 'http://127.0.0.1:18080',
    backend: 'ssh_proxy',
    routeId: 'route-1',
    selectedTransport: 'ssh-native',
    fallbackReason: 'ssh-only',
  };

  assert.equal(remoteStatusMatchesCurrentProxy(status, config(), local()), true);
  assert.deepEqual(appliedProxyFromRemoteStatus(status, config(), local()), {
    local: local(),
    remoteUrl: 'http://127.0.0.1:17903',
    remotePort: 17903,
    remoteBindHost: '127.0.0.1',
    backend: 'ssh_proxy',
    routeId: 'route-1',
    routeOwner: undefined,
    selectedTransport: 'ssh-native',
    connectMode: undefined,
    fallbackReason: 'ssh-only',
  });
});
