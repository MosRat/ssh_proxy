const assert = require('node:assert/strict');
const test = require('node:test');

const {
  healthCheckIntervalMs,
  shouldRunTimedCheck,
} = require('../out/healthMonitor');

test('clamps health check interval to at least one second', () => {
  assert.equal(healthCheckIntervalMs(10), 10_000);
  assert.equal(healthCheckIntervalMs(0), 1_000);
  assert.equal(healthCheckIntervalMs(-5), 1_000);
});

test('decides whether a timed check is due', () => {
  assert.equal(shouldRunTimedCheck(2_000, 1_500, 1_000), false);
  assert.equal(shouldRunTimedCheck(2_500, 1_500, 1_000), true);
});
