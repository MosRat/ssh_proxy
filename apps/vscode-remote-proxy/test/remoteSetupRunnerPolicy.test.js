const assert = require('node:assert/strict');
const test = require('node:test');

const {
  createRemoteSetupFallbackRecord,
  formatRemoteSetupFallbackReason,
  preferredRemoteSetupRunnerKind,
  shouldFallbackRemoteSetup,
} = require('../out/remoteSetupRunnerPolicy');

test('prefers ssh_proxy host exec in auto and explicit ssh_proxy modes', () => {
  assert.equal(preferredRemoteSetupRunnerKind('auto'), 'ssh_proxy_host_exec');
  assert.equal(preferredRemoteSetupRunnerKind('ssh_proxy'), 'ssh_proxy_host_exec');
  assert.equal(preferredRemoteSetupRunnerKind('openssh'), 'openssh');
});

test('allows fallback only for auto ssh_proxy host exec failures', () => {
  assert.equal(shouldFallbackRemoteSetup('auto', 'ssh_proxy_host_exec'), true);
  assert.equal(shouldFallbackRemoteSetup('auto', 'openssh'), false);
  assert.equal(shouldFallbackRemoteSetup('ssh_proxy', 'ssh_proxy_host_exec'), false);
  assert.equal(shouldFallbackRemoteSetup('openssh', 'openssh'), false);
});

test('records remote setup fallback reason with runner names', () => {
  const record = createRemoteSetupFallbackRecord(
    'patch remote settings',
    'ssh_proxy_host_exec',
    'openssh',
    new Error('host exec failed'),
    1234,
  );

  assert.deepEqual(record, {
    label: 'patch remote settings',
    preferred: 'ssh_proxy_host_exec',
    fallback: 'openssh',
    reason: 'host exec failed',
    at: 1234,
  });
  assert.equal(
    formatRemoteSetupFallbackReason(record),
    'patch remote settings: ssh_proxy_host_exec failed; fallback=openssh; reason=host exec failed',
  );
});
