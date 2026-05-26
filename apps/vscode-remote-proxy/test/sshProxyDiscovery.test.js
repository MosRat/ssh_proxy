const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const {
  bundledSshProxyRelativePath,
  describeSshProxyDiscovery,
  resolveSshProxyExecutable,
  resolveSshProxyExecutableCandidates,
  sshProxyUnavailableMessage,
  sshProxyUnavailableCandidatesMessage,
} = require('../out/sshProxyDiscovery');

test('resolves empty ssh_proxy executable from PATH', () => {
  const discovery = resolveSshProxyExecutable('   ');

  assert.deepEqual(discovery, {
    executable: 'ssh_proxy',
    source: 'path',
    configuredValue: '',
  });
  assert.equal(describeSshProxyDiscovery(discovery), 'ssh_proxy from PATH (ssh_proxy)');
  assert.match(sshProxyUnavailableMessage(discovery), /not found on PATH/);
});

test('resolves configured ssh_proxy executable path', () => {
  const discovery = resolveSshProxyExecutable('  "C:/Tools/ssh_proxy.exe"  ');

  assert.deepEqual(discovery, {
    executable: 'C:/Tools/ssh_proxy.exe',
    source: 'configured',
    configuredValue: '"C:/Tools/ssh_proxy.exe"',
  });
  assert.equal(
    describeSshProxyDiscovery(discovery),
    'ssh_proxy from remoteProxy.sshProxy.executable (C:/Tools/ssh_proxy.exe)',
  );
  assert.match(sshProxyUnavailableMessage(discovery), /remoteProxy\.sshProxy\.executable/);
  assert.match(sshProxyUnavailableMessage(discovery), /ssh_proxy release binary/);
});

test('resolves bundled executable before PATH for a supported platform', () => {
  const extensionPath = fs.mkdtempSync(path.join(os.tmpdir(), 'remote-proxy-ext-'));
  const relative = bundledSshProxyRelativePath({ platform: 'win32', arch: 'x64' });
  const executable = path.join(extensionPath, relative);
  fs.mkdirSync(path.dirname(executable), { recursive: true });
  fs.writeFileSync(executable, '');

  const discovery = resolveSshProxyExecutable('ssh_proxy', {
    extensionPath,
    platform: 'win32',
    arch: 'x64',
  });

  assert.deepEqual(discovery, {
    executable,
    source: 'bundled',
    configuredValue: 'ssh_proxy',
  });
  assert.equal(describeSshProxyDiscovery(discovery), `bundled ssh_proxy (${executable})`);
  assert.match(sshProxyUnavailableMessage(discovery), /bundled extension binary/);
});

test('keeps PATH fallback after a bundled candidate', () => {
  const extensionPath = fs.mkdtempSync(path.join(os.tmpdir(), 'remote-proxy-ext-'));
  const relative = bundledSshProxyRelativePath({ platform: 'win32', arch: 'x64' });
  const executable = path.join(extensionPath, relative);
  fs.mkdirSync(path.dirname(executable), { recursive: true });
  fs.writeFileSync(executable, '');

  const candidates = resolveSshProxyExecutableCandidates('ssh_proxy', {
    extensionPath,
    platform: 'win32',
    arch: 'x64',
  });

  assert.deepEqual(candidates, [
    {
      executable,
      source: 'bundled',
      configuredValue: 'ssh_proxy',
    },
    {
      executable: 'ssh_proxy',
      source: 'path',
      configuredValue: 'ssh_proxy',
    },
  ]);
  assert.match(sshProxyUnavailableCandidatesMessage(candidates), /bundled extension binary or PATH/);
});
