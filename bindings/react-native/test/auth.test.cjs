const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { applyAuth, defaultAuth } = require('../src/index.js');

test('auth memory only', () => {
  const st = applyAuth(defaultAuth(), { base_url: 'http://127.0.0.1:8899', token: 't' });
  assert.equal(st.token, 't');
});

test('auth does not write config.yml', () => {
  const prev = process.env.ARIA_COMPUTE_HOME;
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'aria-router-rn-'));
  process.env.ARIA_COMPUTE_HOME = home;
  try {
    applyAuth(defaultAuth(), { token: 't' });
    assert.equal(fs.existsSync(path.join(home, 'config.yml')), false);
  } finally {
    if (prev === undefined) delete process.env.ARIA_COMPUTE_HOME;
    else process.env.ARIA_COMPUTE_HOME = prev;
  }
});
