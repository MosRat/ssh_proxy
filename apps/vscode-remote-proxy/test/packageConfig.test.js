const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const manifest = require('../package.json');

const ROOT = path.resolve(__dirname, '..');

const requiredSettings = {
  'remoteProxy.backend': {
    type: 'string',
    default: 'auto',
    enum: ['auto', 'ssh_proxy', 'openssh'],
    configRead: "config.get<'auto' | 'ssh_proxy' | 'openssh'>('backend', 'auto')",
    typeField: 'readonly backend: ForwardingBackendKind;',
  },
  'remoteProxy.sshProxy.executable': {
    type: 'string',
    default: 'ssh_proxy',
    configRead: "config.get<string>('sshProxy.executable', 'ssh_proxy')",
    typeField: 'readonly sshProxyExecutable: string;',
  },
  'remoteProxy.sshProxy.autoInstallLocalService': {
    type: 'boolean',
    default: true,
    configRead: "config.get<boolean>('sshProxy.autoInstallLocalService', true)",
    typeField: 'readonly sshProxyAutoInstallLocalService: boolean;',
  },
  'remoteProxy.sshProxy.connectMode': {
    type: 'string',
    default: 'reverse-link',
    enum: ['auto', 'reverse-link', 'direct'],
    configRead: "config.get<'auto' | 'reverse-link' | 'direct'>('sshProxy.connectMode', 'reverse-link')",
    typeField: 'readonly sshProxyConnectMode: SshProxyConnectMode;',
  },
  'remoteProxy.sshProxy.routeVolatile': {
    type: 'boolean',
    default: true,
    configRead: "config.get<boolean>('sshProxy.routeVolatile', true)",
    typeField: 'readonly sshProxyRouteVolatile: boolean;',
  },
  'remoteProxy.sshProxy.remoteSetup': {
    type: 'string',
    default: 'auto',
    enum: ['auto', 'ssh_proxy', 'openssh'],
    configRead: "config.get<'auto' | 'ssh_proxy' | 'openssh'>('sshProxy.remoteSetup', 'auto')",
    typeField: 'readonly sshProxyRemoteSetup: RemoteSetupMode;',
  },
  'remoteProxy.forward.healthCheckFailureThreshold': {
    type: 'number',
    default: 2,
    configRead: "config.get<number>('forward.healthCheckFailureThreshold', 2)",
    typeField: 'readonly healthCheckFailureThreshold: number;',
  },
  'remoteProxy.singleton.startLockTimeoutSeconds': {
    type: 'number',
    default: 15,
    configRead: "config.get<number>('singleton.startLockTimeoutSeconds', 15)",
    typeField: 'readonly singletonStartLockTimeoutSeconds: number;',
  },
};

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

  assert.ok(typesSource.includes("export type ForwardingBackendKind = 'auto' | 'ssh_proxy' | 'openssh';"));
  assert.ok(typesSource.includes("export type RemoteSetupMode = 'auto' | 'ssh_proxy' | 'openssh';"));
  assert.ok(typesSource.includes("export type SshProxyConnectMode = 'auto' | 'reverse-link' | 'direct';"));
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
