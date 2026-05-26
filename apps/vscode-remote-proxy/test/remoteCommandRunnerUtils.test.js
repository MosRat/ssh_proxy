const assert = require('node:assert/strict');
const test = require('node:test');

const {
  assertSshProxyHostExecSucceeded,
  buildOpenSshRemoteScriptArgs,
  buildSshProxyHostExecHostArgs,
  sshProxyHostExecStdout,
  sshProxyHostExecTimeoutSecs,
} = require('../out/remoteCommandRunnerUtils');

function config(overrides = {}) {
  return {
    sshArgs: ['-F', 'C:/Users/me/.ssh/config'],
    sshBatchMode: true,
    sshConnectTimeout: 12,
    ...overrides,
  };
}

test('builds OpenSSH remote script args with user args and batch mode', () => {
  assert.deepEqual(
    buildOpenSshRemoteScriptArgs(config(), 'edge'),
    [
      '-F',
      'C:/Users/me/.ssh/config',
      '-o',
      'BatchMode=yes',
      '-o',
      'ConnectTimeout=12',
      'edge',
      'sh',
      '-s',
    ],
  );
});

test('builds OpenSSH args without batch mode and clamps timeout', () => {
  assert.deepEqual(
    buildOpenSshRemoteScriptArgs(config({ sshBatchMode: false, sshConnectTimeout: 0, sshArgs: [] }), 'edge'),
    ['-o', 'ConnectTimeout=1', 'edge', 'sh', '-s'],
  );
});

test('maps SSH args to ssh_proxy host exec --ssh-arg pairs', () => {
  assert.deepEqual(
    buildSshProxyHostExecHostArgs(config({ sshArgs: ['-J', 'jump', '-p', '2222'] })),
    ['--ssh-arg', '-J', '--ssh-arg', 'jump', '--ssh-arg', '-p', '--ssh-arg', '2222'],
  );
});

test('uses SSH connect timeout as host exec timeout budget', () => {
  assert.equal(sshProxyHostExecTimeoutSecs(config({ sshConnectTimeout: 12 })), 12);
  assert.equal(sshProxyHostExecTimeoutSecs(config({ sshConnectTimeout: 0 })), 1);
});

test('accepts successful ssh_proxy host exec results', () => {
  assert.doesNotThrow(() => assertSshProxyHostExecSucceeded({ ok: true, exit_code: 0, timed_out: false }, 'setup'));
  assert.equal(sshProxyHostExecStdout({ ok: true, exit_code: 0, stdout: 'status\n' }, 'setup'), 'status\n');
});

test('rejects missing failed and timed out ssh_proxy host exec results', () => {
  assert.throws(
    () => assertSshProxyHostExecSucceeded(null, 'setup'),
    /setup failed through ssh_proxy host exec: missing JSON result/,
  );
  assert.throws(
    () => assertSshProxyHostExecSucceeded({ ok: false, exit_code: 7, stderr: 'bad remote' }, 'setup'),
    /setup failed through ssh_proxy host exec: exit_code=7 bad remote/,
  );
  assert.throws(
    () => assertSshProxyHostExecSucceeded({ ok: false, exit_code: null, timed_out: true, stderr: 'timeout' }, 'setup'),
    /setup timed out through ssh_proxy host exec: exit_code=null timed_out=true timeout/,
  );
});
