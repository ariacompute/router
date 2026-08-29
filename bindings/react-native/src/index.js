/** @ariacompute/router-rn — auth client; native FFI on-device. */

function defaultAuth() {
  return { base_url: '', token: '' };
}

function applyAuth(existing, updates) {
  const out = { ...existing };
  for (const [k, v] of Object.entries(updates || {})) {
    if (v !== undefined) out[k] = v;
  }
  return out;
}

module.exports = { defaultAuth, applyAuth };
