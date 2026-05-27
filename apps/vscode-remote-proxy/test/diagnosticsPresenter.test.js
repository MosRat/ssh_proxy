const assert = require('node:assert/strict');
const test = require('node:test');

const {
  remoteForwardFailureLine,
  remoteForwardReachableLine,
  REMOTE_PROXY_DIAGNOSTICS_HEADER,
  REMOTE_PROXY_DIAGNOSTICS_SKIP_LINE,
  shouldVerifyRemoteForward,
} = require('../out/diagnosticsPresenter');

const proxy = {
  remoteUrl: 'http://127.0.0.1:17890/',
};

test('decides when diagnostics should verify the remote forwarded port', () => {
  assert.equal(shouldVerifyRemoteForward('running', proxy, 'edge'), true);
  assert.equal(shouldVerifyRemoteForward('starting', proxy, 'edge'), false);
  assert.equal(shouldVerifyRemoteForward('running', undefined, 'edge'), false);
  assert.equal(shouldVerifyRemoteForward('running', proxy, undefined), false);
});

test('formats diagnostics output lines', () => {
  assert.equal(REMOTE_PROXY_DIAGNOSTICS_HEADER, 'Remote Proxy diagnostics');
  assert.equal(
    REMOTE_PROXY_DIAGNOSTICS_SKIP_LINE,
    'diagnose: forwarder is not running, so remote port verification was skipped.',
  );
  assert.equal(
    remoteForwardReachableLine(proxy),
    'diagnose: remote forwarded port is reachable at http://127.0.0.1:17890/',
  );
  assert.equal(
    remoteForwardFailureLine(new Error('connection refused')),
    'diagnose: remote forwarded port check failed: connection refused',
  );
  assert.equal(
    remoteForwardFailureLine('timeout'),
    'diagnose: remote forwarded port check failed: timeout',
  );
});
