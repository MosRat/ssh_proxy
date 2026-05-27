const assert = require('node:assert/strict');
const test = require('node:test');

const {
  buildRemoteProxyStatusLines,
  describeSshProxyDaemonHealth,
  describeSshProxyRouteHealth,
} = require('../out/statusDisplay');

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
    backend: 'ssh_proxy',
    routeId: 'vscode-remote-proxy-edge',
    routeOwner: 'local',
    selectedTransport: 'ssh-reverse-link',
    connectMode: 'reverse-link',
    fallbackReason: 'ssh-only topology',
    ...overrides,
  };
}

function kernelStatus() {
  return {
    daemonStatus: {
      ok: true,
      daemon: { reachable: true },
      health: {
        listeners: {
          control: { ok: true },
          plain_tcp: { ok: true },
          tls_tcp: { ok: true },
          quic: { configured: false },
        },
      },
    },
    sessionStart: undefined,
    sessionStop: undefined,
    routeState: {
      routeId: 'vscode-remote-proxy-edge',
      owner: 'local',
      selectedTransport: 'ssh-reverse-link',
      connectMode: 'reverse-link',
      fallbackReason: 'ssh-only topology',
      remoteUrl: 'http://127.0.0.1:17890',
      cleanupCommand: 'ssh_proxy down --route-id vscode-remote-proxy-edge',
      health: {
        selected_protocol: 'ssh-native',
        control_health: 'healthy',
        active_connections: 1,
        active_streams: 2,
        open_failures: 0,
      },
      liveRoute: undefined,
    },
    lastRefreshAt: 1000,
  };
}

test('summarizes ssh_proxy daemon health for diagnostics and tooltip', () => {
  assert.equal(
    describeSshProxyDaemonHealth('ssh_proxy', kernelStatus()),
    'ok=true daemon=true control=true plain=true tls=true quic=false',
  );
  assert.equal(describeSshProxyDaemonHealth('ssh_proxy', undefined), 'unknown');
});

test('summarizes ssh_proxy route health from kernel link health', () => {
  assert.equal(
    describeSshProxyRouteHealth('ssh_proxy', kernelStatus()),
    'protocol=ssh-native control=healthy connections=1 streams=2 open_failures=0',
  );
  assert.equal(describeSshProxyRouteHealth('ssh_proxy', undefined), 'unknown');
});

test('builds status lines with backend route transport fallback daemon and error', () => {
  const lines = buildRemoteProxyStatusLines({
    status: 'running',
    backend: 'ssh_proxy',
    remoteName: 'ssh-remote',
    remoteAuthority: 'ssh-remote+edge',
    detectedHost: 'edge',
    detectedSource: 'remote authority',
    detectedConfidence: 'high',
    forwardSshHost: 'edge',
    restartBackoff: 'ready',
    proxy: proxy(),
    kernelStatus: kernelStatus(),
    lastError: 'last failure',
  });

  assert.ok(lines.includes('backend: ssh_proxy'));
  assert.ok(lines.includes('route id: vscode-remote-proxy-edge'));
  assert.ok(lines.includes('route owner: local'));
  assert.ok(lines.includes('selected transport: ssh-reverse-link'));
  assert.ok(lines.includes('connect mode: reverse-link'));
  assert.ok(lines.includes('fallback reason: ssh-only topology'));
  assert.ok(lines.includes('daemon health: ok=true daemon=true control=true plain=true tls=true quic=false'));
  assert.ok(lines.includes('route health: protocol=ssh-native control=healthy connections=1 streams=2 open_failures=0'));
  assert.ok(lines.includes('last error: last failure'));
});
