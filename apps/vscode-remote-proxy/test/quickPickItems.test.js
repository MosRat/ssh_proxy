const assert = require('node:assert/strict');
const test = require('node:test');

const {
  buildLocalProxyPickItems,
  buildSshHostPickItems,
  sshHostPickPlaceholder,
} = require('../out/quickPickItems');

test('builds local proxy pick items with manual entry last', () => {
  const proxy = {
    url: 'http://127.0.0.1:7890/',
    scheme: 'http',
    host: '127.0.0.1',
    port: 7890,
    source: 'manual setting',
  };
  const items = buildLocalProxyPickItems([proxy]);

  assert.equal(items[0].label, 'http://127.0.0.1:7890/');
  assert.equal(items[0].description, 'manual setting');
  assert.equal(items[0].candidate, proxy);
  assert.deepEqual(items[1], {
    label: 'Enter proxy URL...',
    description: 'http://127.0.0.1:<port> or socks5://127.0.0.1:<port>',
  });
});

test('builds ssh host pick items with current selection and manual entry', () => {
  const entries = [
    { alias: 'alpha', source: '~/.ssh/config' },
    { alias: 'beta', source: '~/.ssh/config' },
  ];
  const items = buildSshHostPickItems(entries, 'beta');

  assert.deepEqual(items.map((item) => ({
    label: item.label,
    picked: item.picked,
    manual: item.manual,
  })), [
    { label: 'alpha', picked: false, manual: undefined },
    { label: 'beta', picked: true, manual: undefined },
    { label: 'Enter SSH host...', picked: undefined, manual: true },
  ]);
  assert.equal(items[1].entry, entries[1]);
});

test('describes ssh host placeholder from current selection', () => {
  assert.equal(sshHostPickPlaceholder('edge'), 'Current: edge');
  assert.equal(sshHostPickPlaceholder(undefined), 'Select a Host from ~/.ssh/config or enter one manually');
});
