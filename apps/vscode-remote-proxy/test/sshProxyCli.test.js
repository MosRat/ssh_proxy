const assert = require('node:assert/strict');
const test = require('node:test');

const {
  buildSshProxyServiceStatusArgs,
  buildSshProxyRoutesArgs,
  buildSshProxyStopRouteArgs,
  formatSshProxyCommand,
  normalizeSshProxyExecutable,
  parseSshProxyJson,
  redactSshProxyArgs,
  redactSshProxyText,
} = require('../out/sshProxyCliUtils');

test('normalizes an empty ssh_proxy executable to the PATH command', () => {
  assert.equal(normalizeSshProxyExecutable(''), 'ssh_proxy');
  assert.equal(normalizeSshProxyExecutable('  "C:/Tools/ssh_proxy.exe"  '), 'C:/Tools/ssh_proxy.exe');
});

test('builds JSON command shapes consumed by the extension', () => {
  assert.deepEqual(buildSshProxyServiceStatusArgs(), ['service', '--json', 'status']);
  assert.deepEqual(buildSshProxyRoutesArgs(), ['node', 'control', '--json', 'routes']);
  assert.deepEqual(buildSshProxyStopRouteArgs('route-1'), ['node', 'control', '--json', 'stop-route', 'route-1']);
});

test('redacts token flags and proxy URL credentials', () => {
  assert.deepEqual(
    redactSshProxyArgs([
      'route',
      'edge',
      '--token',
      'secret-token',
      '--remote-token=sample-remote-token',
      '--egress-proxy',
      'http://sample-user:sample-password@127.0.0.1:18080',
    ]),
    [
      'route',
      'edge',
      '--token',
      '<redacted>',
      '--remote-token=<redacted>',
      '--egress-proxy',
      'http://<redacted>@127.0.0.1:18080',
    ],
  );
});

test('redacts credentials in free-form text', () => {
  assert.equal(
    redactSshProxyText('failed to use socks5://sample-token:sample-password@example.test:1080/path'),
    'failed to use socks5://<redacted>@example.test:1080/path',
  );
});

test('formats redacted command lines for logs', () => {
  assert.equal(
    formatSshProxyCommand('C:/Program Files/ssh_proxy.exe', ['route', '--token', 'secret']),
    '"C:/Program Files/ssh_proxy.exe" route --token <redacted>',
  );
});

test('parses ssh_proxy JSON output and reports invalid JSON with context', () => {
  assert.deepEqual(parseSshProxyJson('{"ok":true}\n'), { ok: true });
  assert.equal(parseSshProxyJson(''), null);
  assert.throws(
    () => parseSshProxyJson('not json with http://sample-user:sample-password@example.test', 'route explain'),
    /route explain did not return valid JSON:.*http:\/\/<redacted>@example\.test/s,
  );
});
