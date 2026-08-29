const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { applySetup, defaultSetup } = require('../src/index.js');

test('setup memory only', () => {
  const st = applySetup(defaultSetup(), { base_url: 'http://127.0.0.1:8899', token: 't' });
  assert.equal(st.token, 't');
});

test('setup does not write router.yml', () => {
  const prev = process.env.ARIA_COMPUTE_HOME;
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'aria-router-rn-'));
  process.env.ARIA_COMPUTE_HOME = home;
  try {
    applySetup(defaultSetup(), { token: 't' });
    assert.equal(fs.existsSync(path.join(home, 'router.yml')), false);
    assert.equal(fs.existsSync(path.join(home, 'engine.yml')), false);
    assert.equal(fs.existsSync(path.join(home, 'config.yml')), false);
  } finally {
    if (prev === undefined) delete process.env.ARIA_COMPUTE_HOME;
    else process.env.ARIA_COMPUTE_HOME = prev;
  }
});
