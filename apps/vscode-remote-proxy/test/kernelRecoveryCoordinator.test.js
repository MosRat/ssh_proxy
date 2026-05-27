const assert = require('node:assert/strict');
const test = require('node:test');

const {
  isPermissionDeniedMessage,
  KernelRecoveryCoordinator,
} = require('../out/kernelRecoveryCoordinator');

test('records permission denied service failures in the session cache', () => {
  const coordinator = new KernelRecoveryCoordinator();

  coordinator.recordServiceFailure('ERROR: Access is denied.');

  assert.equal(coordinator.isBlocked('persistent-service-install'), true);
  assert.match(coordinator.reason('persistent-service-install'), /Access is denied/);
});

test('ignores non-permission service failures for retryable recovery', () => {
  const coordinator = new KernelRecoveryCoordinator();

  coordinator.recordServiceFailure('daemon is still booting');

  assert.equal(coordinator.isBlocked('persistent-service-install'), false);
});

test('detects common elevation and privilege failures', () => {
  assert.equal(isPermissionDeniedMessage('requires administrator elevation'), true);
  assert.equal(isPermissionDeniedMessage('temporary route timeout'), false);
});
