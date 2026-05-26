const assert = require('node:assert/strict');
const test = require('node:test');

const {
  buildCleanupScript,
  buildGitConfigScript,
  buildProxyEnv,
  buildReadRemoteStatusFileScript,
  buildRemotePortFreeScript,
  buildRemoteSettingsScript,
  buildRemoteStatusFileScript,
  buildServerEnvSetupScript,
  buildVerifyForwardScript,
  shellQuote,
} = require('../out/remoteSetupScripts');
const {
  parseRemoteProxyStatusFile,
} = require('../out/remoteStatusFile');

test('builds proxy environment with upper and lower case keys', () => {
  assert.deepEqual(buildProxyEnv('http://127.0.0.1:17890', 'localhost,127.0.0.1'), {
    HTTP_PROXY: 'http://127.0.0.1:17890',
    HTTPS_PROXY: 'http://127.0.0.1:17890',
    ALL_PROXY: 'http://127.0.0.1:17890',
    NO_PROXY: 'localhost,127.0.0.1',
    http_proxy: 'http://127.0.0.1:17890',
    https_proxy: 'http://127.0.0.1:17890',
    all_proxy: 'http://127.0.0.1:17890',
    no_proxy: 'localhost,127.0.0.1',
  });
});

test('quotes shell values safely for generated remote scripts', () => {
  assert.equal(shellQuote("a'b"), "'a'\\''b'");
  assert.match(buildVerifyForwardScript("bad'host", 17890), /host='bad'\\''host'/);
  assert.match(buildRemotePortFreeScript('127.0.0.1', 17890), /sock\.connect\(\(host, port\)\)/);
});

test('builds server-env-setup script with managed block markers', () => {
  const script = buildServerEnvSetupScript('.vscode-server', buildProxyEnv('http://127.0.0.1:17890', 'localhost'));

  assert.match(script, /# >>> vscode-remote-proxy >>>/);
  assert.match(script, /export HTTP_PROXY=.*http:\/\/127\.0\.0\.1:17890/);
  assert.match(script, /# <<< vscode-remote-proxy <<</);
  assert.match(script, /server-env-setup/);
});

test('builds Git config script with workspace and global policy', () => {
  const script = buildGitConfigScript({
    proxyUrl: 'http://127.0.0.1:17890',
    workspacePaths: ["/home/me/work space/project"],
    applyGlobal: false,
    applyWorkspace: true,
    forceOverride: true,
  });

  assert.match(script, /replace_arg='--replace-all'/);
  assert.match(script, /global Git proxy config disabled/);
  assert.match(script, /apply_workspace_git '\/home\/me\/work space\/project'/);
});

test('builds remote settings patcher as a reusable script', () => {
  const script = buildRemoteSettingsScript({
    serverDir: '.vscode-server',
    values: {
      'http.proxy': 'http://127.0.0.1:17890',
      'http.proxySupport': 'override',
    },
  });

  assert.match(script, /Machine', 'settings\.json'/);
  assert.match(script, /stripJsonComments/);
  assert.match(script, /payload\.values/);
});

test('builds remote status file and cleanup scripts from inputs', () => {
  const statusScript = buildRemoteStatusFileScript('.vscode-server', {
    proxyUrl: 'http://127.0.0.1:17890',
    bindHost: '127.0.0.1',
    port: 17890,
    updatedAt: '2026-05-26T00:00:00.000Z',
    localProxySource: 'test',
    localProxyUrl: 'http://127.0.0.1:18080',
    backend: 'ssh_proxy',
    routeId: 'route-1',
    selectedTransport: 'ssh-native',
  });
  assert.match(statusScript, /remote-proxy-status\.json/);
  assert.match(statusScript, /chmod 600/);
  assert.deepEqual(decodeStatusPayload(statusScript), {
    proxyUrl: 'http://127.0.0.1:17890',
    bindHost: '127.0.0.1',
    port: 17890,
    updatedAt: '2026-05-26T00:00:00.000Z',
    localProxySource: 'test',
    localProxyUrl: 'http://127.0.0.1:18080',
    backend: 'ssh_proxy',
    routeId: 'route-1',
    selectedTransport: 'ssh-native',
  });

  const cleanupScript = buildCleanupScript('.vscode-server', ['/home/me/project']);
  assert.match(cleanupScript, /remote-proxy-status\.json/);
  assert.match(cleanupScript, /server-env-setup/);
  assert.match(cleanupScript, /cleanup_workspace_git '\/home\/me\/project'/);
  assert.doesNotMatch(cleanupScript, /ssh_proxy\s+service|systemctl|service\s+(stop|disable|uninstall)/);
});

test('builds remote status file reader script', () => {
  const script = buildReadRemoteStatusFileScript('.vscode-server');

  assert.match(script, /remote-proxy-status\.json/);
  assert.match(script, /cat "\$target"/);
  assert.match(script, /exit 0/);
});

test('parses remote proxy status file content', () => {
  assert.equal(parseRemoteProxyStatusFile(''), undefined);
  assert.equal(parseRemoteProxyStatusFile('not json'), undefined);
  assert.deepEqual(parseRemoteProxyStatusFile(JSON.stringify({
    proxyUrl: 'http://127.0.0.1:17890',
    bindHost: '127.0.0.1',
    port: 17890,
    localProxyUrl: 'http://127.0.0.1:18080',
    backend: 'ssh_proxy',
  })), {
    proxyUrl: 'http://127.0.0.1:17890',
    bindHost: '127.0.0.1',
    port: 17890,
    updatedAt: undefined,
    localProxySource: undefined,
    localProxyUrl: 'http://127.0.0.1:18080',
    backend: 'ssh_proxy',
    routeId: undefined,
    routeOwner: undefined,
    selectedTransport: undefined,
    connectMode: undefined,
    fallbackReason: undefined,
  });
});

function decodeStatusPayload(script) {
  const match = script.match(/printf '%s' '([^']+)' \| base64 -d/);
  assert.ok(match, 'status script should write a base64 payload through base64 -d');
  return JSON.parse(Buffer.from(match[1], 'base64').toString('utf8'));
}
