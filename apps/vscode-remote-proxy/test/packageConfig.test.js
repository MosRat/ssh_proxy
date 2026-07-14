const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const manifest = require('../package.json');

const ROOT = path.resolve(__dirname, '..');

const requiredSettings = {
  'remoteProxy.sshProxy.executable': {
    type: 'string',
    default: 'ssh_proxy',
    configRead: "config.get<string>('sshProxy.executable', 'ssh_proxy')",
    typeField: 'readonly sshProxyExecutable: string;',
  },
  'remoteProxy.sshProxy.connectMode': {
    type: 'string',
    default: 'auto',
    enum: ['auto', 'reverse-link', 'direct'],
    configRead: "config.get<'auto' | 'reverse-link' | 'direct'>('sshProxy.connectMode', 'auto')",
    typeField: 'readonly sshProxyConnectMode: SshProxyConnectMode;',
  },
  'remoteProxy.forward.healthCheckFailureThreshold': {
    type: 'number',
    default: 2,
    configRead: "config.get<number>('forward.healthCheckFailureThreshold', 2)",
    typeField: 'readonly healthCheckFailureThreshold: number;',
  },
};

const hiddenCompatibilitySettings = [
  'remoteProxy.backend',
  'remoteProxy.ssh.executable',
  'remoteProxy.ssh.args',
  'remoteProxy.ssh.batchMode',
  'remoteProxy.ssh.connectTimeout',
  'remoteProxy.ssh.serverAliveInterval',
  'remoteProxy.ssh.serverAliveCountMax',
  'remoteProxy.ssh.tcpKeepAlive',
  'remoteProxy.sshProxy.autoInstallLocalService',
  'remoteProxy.sshProxy.allowElevationPrompt',
  'remoteProxy.sshProxy.preferPersistentService',
  'remoteProxy.sshProxy.openSshFallbackPolicy',
  'remoteProxy.sshProxy.brokerMode',
  'remoteProxy.sshProxy.routeVolatile',
  'remoteProxy.sshProxy.remoteSetup',
  'remoteProxy.forward.retryOnExit',
  'remoteProxy.forward.retryDelaySeconds',
  'remoteProxy.singleton.reuseEnabled',
  'remoteProxy.singleton.leaseTtlSeconds',
  'remoteProxy.singleton.startLockTimeoutSeconds',
];

test('contributes ssh_proxy kernel settings with stable defaults', () => {
  const properties = manifest.contributes.configuration.properties;
  for (const [key, expected] of Object.entries(requiredSettings)) {
    const setting = properties[key];
    assert.ok(setting, `${key} should be contributed`);
    assert.equal(setting.type, expected.type, `${key} type`);
    assert.deepEqual(setting.default, expected.default, `${key} default`);
    if (expected.enum) {
      assert.deepEqual(setting.enum, expected.enum, `${key} enum`);
    }
    assert.match(setting.description, /\S/, `${key} description`);
  }
});

test('reads ssh_proxy kernel settings into the typed config object', () => {
  const configSource = fs.readFileSync(path.join(ROOT, 'src', 'config.ts'), 'utf8');
  const typesSource = fs.readFileSync(path.join(ROOT, 'src', 'types.ts'), 'utf8');

  for (const [key, expected] of Object.entries(requiredSettings)) {
    assert.ok(configSource.includes(expected.configRead), `${key} should be read from VS Code config`);
    assert.ok(typesSource.includes(expected.typeField), `${key} should be represented in RemoteProxyConfig`);
  }

  assert.ok(typesSource.includes("export type SshProxyConnectMode = 'auto' | 'reverse-link' | 'direct';"));
});

test('hides legacy fallback settings from the production configuration surface', () => {
  const properties = manifest.contributes.configuration.properties;
  for (const key of hiddenCompatibilitySettings) {
    assert.equal(properties[key], undefined, `${key} should not be user-facing`);
  }
});

test('keeps legacy backend helpers out of the production extension source', () => {
  const sourceFiles = new Set(fs.readdirSync(path.join(ROOT, 'src')));
  const forbiddenFiles = [
    'openSshReverseBackend.ts',
    'sessionDaemon.ts',
    'serviceRecovery.ts',
    'remoteSetupScripts.ts',
    'leaseCoordinator.ts',
    'routeArgsBuilder.ts',
  ];

  for (const file of forbiddenFiles) {
    assert.equal(sourceFiles.has(file), false, `${file} should not return to the normal extension path`);
  }
});

test('keeps explicit scripts for staging bundled ssh_proxy binaries', () => {
  assert.equal(
    manifest.scripts['kernel:stage'],
    'pwsh -NoProfile -ExecutionPolicy Bypass -File ../../scripts/stage-vscode-extension-binaries.ps1',
  );
  assert.equal(manifest.scripts['package:with-kernel'], 'npm run kernel:stage && vsce package');
});

test('declares monorepo publishing metadata', () => {
  assert.equal(manifest.license, 'MIT');
  assert.equal(manifest.icon, 'assets/icon.png');
  assert.ok(fs.existsSync(path.join(ROOT, manifest.icon)), 'extension icon should exist');
  assert.deepEqual(manifest.repository, {
    type: 'git',
    url: 'https://github.com/MosRat/ssh_proxy.git',
    directory: 'apps/vscode-remote-proxy',
  });
  assert.equal(manifest.bugs.url, 'https://github.com/MosRat/ssh_proxy/issues');
});

test('keeps lockfile aligned with package metadata', () => {
  const lock = JSON.parse(fs.readFileSync(path.join(ROOT, 'package-lock.json'), 'utf8'));
  assert.equal(lock.version, manifest.version);
  assert.equal(lock.packages[''].version, manifest.version);
  assert.equal(lock.packages[''].license, manifest.license);
  assert.ok(!JSON.stringify(lock).includes('registry.npmmirror.com'));
});
