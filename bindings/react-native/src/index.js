/** @ariacompute/router-rn — auth client; native FFI on-device. */

function defaultSetup() {
  return { base_url: '', token: '' };
}

function applySetup(existing, updates) {
  const out = { ...existing };
  for (const [k, v] of Object.entries(updates || {})) {
    if (v !== undefined) out[k] = v;
  }
  return out;
}

module.exports = { defaultSetup, applySetup };
