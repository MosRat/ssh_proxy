const assert = require('node:assert/strict');
const test = require('node:test');

const {
  shouldReleaseOwnedLease,
  shouldStopSshProxyRoute,
} = require('../out/routeOwnership');

test('stops ssh_proxy routes only when this window owns them', () => {
  assert.equal(shouldStopSshProxyRoute('route-1', true), true);
  assert.equal(shouldStopSshProxyRoute('route-1', false), false);
  assert.equal(shouldStopSshProxyRoute(undefined, true), false);
});

test('releases only owned leases with a known target key', () => {
  assert.equal(shouldReleaseOwnedLease('owner', 'edge'), true);
  assert.equal(shouldReleaseOwnedLease('shared', 'edge'), false);
  assert.equal(shouldReleaseOwnedLease('none', 'edge'), false);
  assert.equal(shouldReleaseOwnedLease('owner', undefined), false);
});
