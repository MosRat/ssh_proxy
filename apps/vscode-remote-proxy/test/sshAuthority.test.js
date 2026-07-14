const assert = require('node:assert/strict');
const test = require('node:test');

const { parseSshAuthority, parseSshHostAuthority } = require('../out/sshAuthority');

test('decodes the hex JSON authority emitted by Remote-SSH', () => {
  const encoded = Buffer.from(JSON.stringify({ hostName: '80G_4_new' }), 'utf8').toString('hex');
  assert.deepEqual(parseSshAuthority(`ssh-remote+${encoded}`), {
    host: '80G_4_new',
    authority: `ssh-remote+${encoded}`,
  });
});

test('parses URI-prefixed and URL-encoded SSH authorities', () => {
  assert.deepEqual(parseSshAuthority('vscode-remote://ssh-remote+user%40edge/path'), {
    host: 'user@edge',
    authority: 'ssh-remote+user%40edge',
  });
});

test('preserves ordinary and hex-looking SSH config aliases', () => {
  assert.equal(parseSshAuthority('ssh-remote+102')?.host, '102');
  assert.equal(parseSshAuthority('ssh-remote+deadbeef')?.host, 'deadbeef');
  assert.equal(parseSshHostAuthority('legacy-alias'), 'legacy-alias');
});

test('rejects a decoded JSON envelope without a hostName', () => {
  const encoded = Buffer.from(JSON.stringify({ hostname: 'wrong-field' }), 'utf8').toString('hex');
  assert.equal(parseSshAuthority(`ssh-remote+${encoded}`), undefined);
});

test('rejects a damaged hex JSON envelope instead of using it as a hostname', () => {
  const encoded = Buffer.from('{"hostName":', 'utf8').toString('hex');
  assert.equal(parseSshAuthority(`ssh-remote+${encoded}`), undefined);
});

test('rejects non-SSH and empty authorities', () => {
  assert.equal(parseSshAuthority('wsl+Ubuntu'), undefined);
  assert.equal(parseSshAuthority('ssh-remote+'), undefined);
});
