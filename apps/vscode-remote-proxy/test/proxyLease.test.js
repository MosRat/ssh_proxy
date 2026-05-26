const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const {
  ProxyLeaseManager,
} = require('../out/proxyLease');

function output() {
  return {
    appendLine() {},
  };
}

function proxy() {
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
    routeId: 'route-1',
  };
}

async function tempLeaseRoot() {
  return fs.mkdtemp(path.join(os.tmpdir(), 'remote-proxy-lease-'));
}

test('writes leases atomically and preserves same-owner startedAt', async () => {
  const leaseRoot = await tempLeaseRoot();
  const manager = new ProxyLeaseManager(output(), 'owner-1', { leaseRoot });

  await manager.write('Edge', 'edge', proxy());
  const first = await manager.read('edge');
  assert.ok(first);
  assert.equal(first.ownerId, 'owner-1');
  assert.equal(first.backend, 'ssh_proxy');
  assert.equal(first.routeId, 'route-1');

  await manager.write('edge', 'edge', proxy());
  const second = await manager.read('edge');
  assert.equal(second.startedAt, first.startedAt);
  assert.ok(second.updatedAt >= first.updatedAt);
});

test('serializes concurrent starts with a per-target lock', async () => {
  const leaseRoot = await tempLeaseRoot();
  const first = new ProxyLeaseManager(output(), 'owner-1', { leaseRoot });
  const second = new ProxyLeaseManager(output(), 'owner-2', { leaseRoot });

  const lock = await first.acquireStartLock('edge', 100, 10_000);
  await assert.rejects(
    () => second.acquireStartLock('edge', 25, 10_000),
    /another VS Code window is still starting Remote Proxy/,
  );
  await lock.release();

  const secondLock = await second.acquireStartLock('edge', 100, 10_000);
  await secondLock.release();
});

test('takes over stale start locks', async () => {
  const leaseRoot = await tempLeaseRoot();
  const first = new ProxyLeaseManager(output(), 'owner-1', { leaseRoot });
  const second = new ProxyLeaseManager(output(), 'owner-2', { leaseRoot });

  const lock = await first.acquireStartLock('edge', 100, 10_000);
  await new Promise((resolve) => setTimeout(resolve, 10));
  const takeover = await second.acquireStartLock('edge', 100, 1);
  await takeover.release();
  await lock.release();
});
