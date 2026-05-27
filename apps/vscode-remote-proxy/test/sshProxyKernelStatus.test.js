const assert = require('node:assert/strict');
const test = require('node:test');

const {
  createSshProxyRouteState,
  findSshProxyLiveRoute,
  isSshProxyOk,
  refreshSshProxyRouteState,
} = require('../out/sshProxyKernelStatus');

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

test('detects explicit ssh_proxy ok status without assuming missing ok is failure', () => {
  assert.equal(isSshProxyOk({ ok: true }), true);
  assert.equal(isSshProxyOk({ ok: false }), false);
  assert.equal(isSshProxyOk({ plan: {} }), undefined);
});

test('extracts route state from route start JSON', () => {
  const state = createSshProxyRouteState(
    {
      route_id: 'vscode-remote-proxy-edge',
      owner: 'local',
      selected_transport: 'ssh-reverse-link',
      connect_mode: 'reverse-link',
      remote_url: 'http://127.0.0.1:17890',
      fallback_reason: 'ssh-only topology',
      cleanup_command: 'ssh_proxy down --route-id vscode-remote-proxy-edge',
      health: { state: 'starting' },
    },
    proxy(),
    'reverse-link',
  );

  assert.equal(state.routeId, 'vscode-remote-proxy-edge');
  assert.equal(state.owner, 'local');
  assert.equal(state.selectedTransport, 'ssh-reverse-link');
  assert.equal(state.connectMode, 'reverse-link');
  assert.equal(state.fallbackReason, 'ssh-only topology');
  assert.equal(state.remoteUrl, 'http://127.0.0.1:17890');
  assert.deepEqual(state.health, { state: 'starting' });
});

test('falls back to deterministic route id and cleanup command', () => {
  const state = createSshProxyRouteState({}, proxy(), 'reverse-link');

  assert.match(state.routeId, /^vscode-remote-proxy-[0-9a-f]+$/);
  assert.equal(state.cleanupCommand, `ssh_proxy down --route-id ${state.routeId}`);
});

test('finds and merges live route health from daemon route output', () => {
  const state = createSshProxyRouteState(
    {
      route_id: 'route-1',
      selected_transport: 'tls-tcp',
    },
    proxy(),
    'reverse-link',
  );
  const routes = {
    ok: true,
    routes: [
      {
        id: 'route-1',
        runtime: {
          selected_transport: 'quic-native',
          fallback_reason: 'direct QUIC selected',
        },
        link: {
          health: {
            selected_protocol: 'quic-native',
            active_connections: 1,
            open_failures: 0,
          },
        },
      },
    ],
  };

  assert.equal(findSshProxyLiveRoute(routes, 'route-1').id, 'route-1');
  const refreshed = refreshSshProxyRouteState(state, routes);
  assert.equal(refreshed.selectedTransport, 'quic-native');
  assert.equal(refreshed.fallbackReason, 'direct QUIC selected');
  assert.deepEqual(refreshed.health, {
    selected_protocol: 'quic-native',
    active_connections: 1,
    open_failures: 0,
  });
});
