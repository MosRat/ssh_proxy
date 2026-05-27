const assert = require('node:assert/strict');
const Module = require('node:module');
const test = require('node:test');

const originalLoad = Module._load;
Module._load = function patchedLoad(request, parent, isMain) {
  if (request === 'vscode') {
    return {
      ConfigurationTarget: { Global: 1 },
      env: { appName: 'VS Code' },
      workspace: {
        getConfiguration() {
          return {
            get() {
              return {};
            },
            update: async () => {},
          };
        },
      },
    };
  }
  return originalLoad.call(this, request, parent, isMain);
};
const { RemoteSetup } = require('../out/remoteSetup');
Module._load = originalLoad;

test('verifyForwardReady retries until the forwarded port becomes ready', async () => {
  const logs = [];
  let attempts = 0;

  class TestRemoteSetup extends RemoteSetup {
    constructor(output) {
      super(output);
    }

    async verifyForward() {
      attempts += 1;
      if (attempts < 2) {
        throw new Error('not ready yet');
      }
    }
  }

  const setup = new TestRemoteSetup({
    appendLine(line) {
      logs.push(line);
    },
  });

  await setup.verifyForwardReady(
    {},
    'host-1',
    { remoteBindHost: '127.0.0.1', remotePort: 17890 },
    { timeoutMs: 100, pollMs: 1 },
  );

  assert.equal(attempts, 2);
  assert.match(logs.join('\n'), /became ready after 2 attempts/);
});
