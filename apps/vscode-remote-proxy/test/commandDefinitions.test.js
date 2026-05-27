const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const manifest = require('../package.json');
const { REMOTE_PROXY_COMMANDS } = require('../out/commandDefinitions');

const ROOT = path.resolve(__dirname, '..');

test('keeps command definitions aligned with package contributions', () => {
  const commands = REMOTE_PROXY_COMMANDS.map(({ command }) => command);
  const contributed = manifest.contributes.commands.map(({ command }) => command);
  const activationEvents = manifest.activationEvents
    .filter((event) => event.startsWith('onCommand:'))
    .map((event) => event.slice('onCommand:'.length));

  assert.deepEqual(contributed, commands);
  assert.deepEqual(activationEvents, commands);
});

test('keeps command registry listening only to remoteProxy configuration', () => {
  const source = fs.readFileSync(path.join(ROOT, 'src', 'commandRegistry.ts'), 'utf8');

  assert.match(source, /REMOTE_PROXY_COMMANDS\.map/);
  assert.match(source, /affectsConfiguration\('remoteProxy'\)/);
});
