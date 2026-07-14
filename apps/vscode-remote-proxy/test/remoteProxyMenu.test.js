const assert = require('node:assert/strict');
const test = require('node:test');

const {
  buildRemoteProxyMenuItems,
} = require('../out/remoteProxyMenu');

function actions(calls) {
  return {
    start: () => calls.push('start'),
    restart: () => calls.push('restart'),
    stop: () => calls.push('stop'),
    diagnose: () => calls.push('diagnose'),
    applySettingsOnly: () => calls.push('applySettingsOnly'),
    cleanupRemote: () => calls.push('cleanupRemote'),
    pickLocalProxy: () => calls.push('pickLocalProxy'),
    pickSshHost: () => calls.push('pickSshHost'),
    clearSshHost: () => calls.push('clearSshHost'),
    openSettings: () => calls.push('openSettings'),
    showOutput: () => calls.push('showOutput'),
  };
}

test('builds stable menu labels and stopped start action', () => {
  const calls = [];
  const items = buildRemoteProxyMenuItems('stopped', {}, actions(calls));

  assert.deepEqual(items.map((item) => item.label), [
    '$(play) Start',
    '$(debug-stop) Stop',
    '$(pulse) Diagnose',
    '$(gear) Apply Remote Settings',
    '$(trash) Clean Remote Settings',
    '$(plug) Pick Local Proxy',
    '$(server) Pick SSH Host',
    '$(close) Clear SSH Host Override',
    '$(settings-gear) Open Settings',
    '$(output) Show Output',
  ]);
  assert.equal(items[0].description, 'Start proxy forwarding');
  items[0].run();
  assert.deepEqual(calls, ['start']);
});

test('builds running restart action and proxy-aware apply description', () => {
  const calls = [];
  const items = buildRemoteProxyMenuItems('running', {
    proxy: {
      remoteUrl: 'http://127.0.0.1:17890',
    },
  }, actions(calls));

  assert.equal(items[0].label, '$(debug-restart) Restart');
  assert.equal(items[0].description, 'Rebuild the SSH reverse tunnel');
  assert.equal(items[3].description, 'Write http://127.0.0.1:17890 to remote VS Code, terminal, and Git settings');
  items[0].run();
  items[3].run();
  assert.deepEqual(calls, ['restart', 'applySettingsOnly']);
});
