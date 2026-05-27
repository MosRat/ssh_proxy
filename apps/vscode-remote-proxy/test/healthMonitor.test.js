const assert = require('node:assert/strict');
const test = require('node:test');

const {
  decideRestartBackoff,
  healthCheckIntervalMs,
  leaseHeartbeatIntervalMs,
  shouldRunTimedCheck,
} = require('../out/healthMonitor');

test('calculates heartbeat interval from lease ttl and health interval', () => {
  assert.equal(leaseHeartbeatIntervalMs(20, 10), 10_000);
  assert.equal(leaseHeartbeatIntervalMs(1, 10), 1_000);
  assert.equal(leaseHeartbeatIntervalMs(300, 10), 10_000);
});

test('clamps health check interval to at least one second', () => {
  assert.equal(healthCheckIntervalMs(10), 10_000);
  assert.equal(healthCheckIntervalMs(0), 1_000);
  assert.equal(healthCheckIntervalMs(-5), 1_000);
});

test('decides whether a timed check is due', () => {
  assert.equal(shouldRunTimedCheck(2_000, 1_500, 1_000), false);
  assert.equal(shouldRunTimedCheck(2_500, 1_500, 1_000), true);
});

test('applies restart backoff and preserves active wait windows', () => {
  assert.deepEqual(decideRestartBackoff(10_000, 15_100, 2, 30), {
    canRestart: false,
    restartFailures: 2,
    nextRestartAt: 15_100,
    waitSeconds: 6,
  });

  assert.deepEqual(decideRestartBackoff(20_000, 15_100, 2, 30), {
    canRestart: true,
    restartFailures: 3,
    nextRestartAt: 40_000,
  });

  assert.deepEqual(decideRestartBackoff(50_000, 40_000, 10, 30), {
    canRestart: true,
    restartFailures: 11,
    nextRestartAt: 80_000,
  });
});
