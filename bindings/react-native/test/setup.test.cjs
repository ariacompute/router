const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { applySetup, defaultSetup, Router } = require('../src/index.js');

const ffi = process.env.ARIA_ROUTER_FFI_LIB;
const cfg = process.env.ARIA_ROUTER_CONFIG;
const hasFfi = Boolean(ffi && cfg);

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

test('Router setup memory', () => {
  const r = new Router();
  r.setup({ base_url: 'http://127.0.0.1:8899', token: 't' });
  assert.equal(r.setupStatus().token, 't');
  r.setupClear();
  assert.equal(r.setupStatus().token, '');
});

test('init_ok', { skip: !hasFfi }, () => {
  const r = new Router().init(cfg);
  try {
    const m = r.models();
    assert.ok(JSON.stringify(m).includes('semantic-auto'));
  } finally {
    r.close();
  }
});

test('complete_ok', { skip: !hasFfi }, () => {
  const r = new Router().init(cfg);
  try {
    const out = r.complete(
      [{ role: 'user', content: 'hi' }],
      { model: 'ariacompute/semantic-auto' },
    );
    assert.ok(JSON.stringify(out).includes('hello-from-router'));
  } finally {
    r.close();
  }
});
