const assert = require('node:assert/strict');
const test = require('node:test');

const {
  clampHealthFailureThreshold,
  isDeterministicPortFailureMessage,
  recordHealthCheckFailure,
  shouldRetryForwardAttempt,
} = require('../out/reliabilityPolicy');

test('requires consecutive health failures before restart', () => {
  assert.deepEqual(recordHealthCheckFailure(0, 2), {
    failures: 1,
    threshold: 2,
    shouldRestart: false,
  });
  assert.deepEqual(recordHealthCheckFailure(1, 2), {
    failures: 2,
    threshold: 2,
    shouldRestart: true,
  });
});

test('clamps health failure threshold', () => {
  assert.equal(clampHealthFailureThreshold(0), 1);
  assert.equal(clampHealthFailureThreshold(3.6), 4);
  assert.equal(clampHealthFailureThreshold(99), 10);
  assert.equal(clampHealthFailureThreshold(Number.NaN), 2);
});

test('detects deterministic port conflicts across backends', () => {
  assert.equal(isDeterministicPortFailureMessage('remote port forwarding failed for listen port 17890'), true);
  assert.equal(isDeterministicPortFailureMessage('EADDRINUSE: address already in use'), true);
  assert.equal(isDeterministicPortFailureMessage('temporary network timeout'), false);
  assert.equal(shouldRetryForwardAttempt({ deterministic: true, message: 'custom deterministic failure' }), true);
  assert.equal(shouldRetryForwardAttempt(new Error('bind: address already in use')), true);
  assert.equal(shouldRetryForwardAttempt(new Error('permission denied while connecting')), false);
});
