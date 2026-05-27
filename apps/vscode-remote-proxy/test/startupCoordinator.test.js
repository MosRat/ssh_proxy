const assert = require('node:assert/strict');
const test = require('node:test');

const {
  checkApplySettingsPreflight,
  checkCleanupPreflight,
  checkStartPreflight,
  planAutoStart,
} = require('../out/startupCoordinator');

function config(overrides = {}) {
  return {
    enabled: true,
    autoStart: true,
    sshHostOverride: '',
    ...overrides,
  };
}

function remote(overrides = {}) {
  return {
    kind: 'ssh',
    name: 'ssh-remote',
    authority: 'ssh-remote+edge',
    sshHost: 'edge',
    sshHostSource: 'vscode-remote URI authority',
    ...overrides,
  };
}

test('plans auto-start only for enabled SSH remotes', () => {
  assert.deepEqual(planAutoStart(config(), remote()), { action: 'start' });
  assert.deepEqual(planAutoStart(config({ enabled: false }), remote()), {
    action: 'skip',
    outputLine: 'Remote Proxy is disabled or autoStart is off.',
  });
  assert.deepEqual(planAutoStart(config(), remote({ kind: 'none', name: undefined, authority: undefined, sshHost: undefined, sshHostSource: undefined })), {
    action: 'skip',
    outputLine: 'Not in a remote window; auto-start skipped.',
  });
  assert.deepEqual(planAutoStart(config(), remote({ kind: 'wsl', name: 'wsl', sshHost: undefined, sshHostSource: undefined })), {
    action: 'skip',
    outputLine: 'Remote kind "wsl" detected; only SSH is currently auto-started.',
    statusText: '$(circle-slash) Proxy wsl',
  });
});

test('checks interactive start preflight without VS Code side effects', () => {
  assert.deepEqual(checkStartPreflight(config(), remote()), { action: 'continue' });
  assert.deepEqual(checkStartPreflight(config({ enabled: false }), remote()), {
    action: 'disabled',
    informationMessage: 'Remote Proxy is disabled.',
  });
  assert.deepEqual(checkStartPreflight(config(), remote({ kind: 'wsl', name: 'wsl', sshHost: undefined, sshHostSource: undefined })), {
    action: 'unsupported-remote',
    warningMessage: 'Remote Proxy currently supports automatic forwarding for Remote SSH windows.',
    outputLine: 'Cannot start: remote={"kind":"wsl","name":"wsl","authority":"ssh-remote+edge"}',
  });
  assert.deepEqual(checkStartPreflight(config({ sshHostOverride: 'edge' }), remote({ kind: 'wsl', name: 'wsl', sshHost: undefined, sshHostSource: undefined })), {
    action: 'continue',
  });
});

test('checks cleanup and apply remote action preflight', () => {
  const unsupported = remote({ kind: 'wsl', name: 'wsl', sshHost: undefined, sshHostSource: undefined });

  assert.deepEqual(checkCleanupPreflight(config(), remote()), { action: 'continue' });
  assert.deepEqual(checkApplySettingsPreflight(config(), remote()), { action: 'continue' });
  assert.deepEqual(checkCleanupPreflight(config(), unsupported), {
    action: 'unsupported-remote',
    warningMessage: 'Remote Proxy cleanup requires a Remote SSH window or remoteProxy.ssh.host.',
  });
  assert.deepEqual(checkApplySettingsPreflight(config(), unsupported), {
    action: 'unsupported-remote',
    warningMessage: 'Remote Proxy settings currently require a Remote SSH window or remoteProxy.ssh.host.',
  });
  assert.deepEqual(checkCleanupPreflight(config({ sshHostOverride: 'edge' }), unsupported), { action: 'continue' });
  assert.deepEqual(checkApplySettingsPreflight(config({ sshHostOverride: 'edge' }), unsupported), { action: 'continue' });
});
