/**
 * @ariacompute/router-rn — in-memory setup helpers + Router over native module or host koffi.
 */
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

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

function ariaHome() {
  return process.env.ARIA_COMPUTE_HOME || path.join(os.homedir(), '.ariacompute');
}

function ffiLibNames() {
  if (process.platform === 'win32') return ['aria-router_ffi.dll', 'aria_router_ffi.dll'];
  if (process.platform === 'darwin') {
    return ['libaria-router_ffi.dylib', 'libaria_router_ffi.dylib'];
  }
  return ['libaria-router_ffi.so', 'libaria_router_ffi.so'];
}

function firstExisting(dir) {
  for (const name of ffiLibNames()) {
    const p = path.join(dir, name);
    if (fs.existsSync(p)) return p;
  }
  return null;
}

function resolveLibPath(explicit) {
  if (explicit && fs.existsSync(explicit)) return explicit;
  const env = process.env.ARIA_ROUTER_FFI_LIB;
  if (env && fs.existsSync(env)) return env;
  const bundled = firstExisting(path.join(__dirname, '..', 'lib'));
  if (bundled) return bundled;
  const cached = firstExisting(path.join(ariaHome(), 'lib'));
  if (cached) return cached;
  throw new Error('libaria-router_ffi not found; set ARIA_ROUTER_FFI_LIB');
}

function expandHome(p) {
  if (!p) return p;
  if (p === '~') return os.homedir();
  if (p.startsWith('~/')) return path.join(os.homedir(), p.slice(2));
  return p;
}

function parseJsonBuf(buf) {
  const text = buf.toString('utf8').replace(/\0+$/, '');
  if (!text) return {};
  return JSON.parse(text);
}

function tryNativeModule() {
  try {
    const rn = require('react-native');
    return rn?.NativeModules?.AriaRouter || null;
  } catch {
    return null;
  }
}

function createKoffiBackend() {
  let koffi;
  try {
    koffi = require('koffi');
  } catch (e) {
    throw new Error('koffi required for host FFI: ' + (e && e.message));
  }
  const lib = koffi.load(resolveLibPath());
  return {
    init: lib.func('aria_router_init', 'void*', ['str']),
    connect: lib.func('aria_router_connect', 'void*', ['str']),
    destroy: lib.func('aria_router_destroy', 'void', ['void*']),
    setup: lib.func('aria_router_setup', 'void', ['void*', 'str', 'str']),
    complete: lib.func('aria_router_complete', 'int', ['void*', 'str', 'str', 'void*', 'size_t']),
    models: lib.func('aria_router_models', 'int', ['void*', 'void*', 'size_t']),
    lastRoute: lib.func('aria_router_last_route', 'int', ['void*', 'void*', 'size_t']),
    lastError: lib.func('aria_router_last_error', 'str', []),
  };
}

class Router {
  constructor() {
    this._auth = defaultSetup();
    this._native = tryNativeModule();
    this._fn = null;
    this._handle = null;
  }

  setup(u = {}) {
    this._auth = applySetup(this._auth, u);
    if (this._handle && this._fn?.setup) {
      this._fn.setup(
        this._handle,
        u.base_url !== undefined ? u.base_url : null,
        u.token !== undefined ? u.token : null,
      );
    }
    return this;
  }

  setupStatus() {
    return { ...this._auth };
  }

  setupClear() {
    this._auth = defaultSetup();
    if (this._handle && this._fn?.setup) {
      this._fn.setup(this._handle, '', '');
    }
    return this;
  }

  _ensureHost() {
    if (this._native) return 'native';
    if (typeof process !== 'undefined' && process.env && process.env.ARIA_ROUTER_FFI_LIB) {
      if (!this._fn) this._fn = createKoffiBackend();
      return 'koffi';
    }
    throw new Error('native AriaRouter module not linked');
  }

  _err(fallback) {
    if (this._fn) return this._fn.lastError() || fallback;
    return fallback;
  }

  _syncAuthIfSet() {
    if (!this._handle || !this._fn?.setup) return;
    if (this._auth.base_url || this._auth.token) {
      this._fn.setup(
        this._handle,
        this._auth.base_url || null,
        this._auth.token || null,
      );
    }
  }

  init(configPath) {
    const mode = this._ensureHost();
    if (mode === 'native') {
      this._native.init(configPath || null);
      this._native.setup?.(this._auth.base_url || null, this._auth.token || null);
      return this;
    }
    if (this._handle) this.close();
    let pathArg = null;
    if (configPath != null && String(configPath).trim() !== '') {
      pathArg = expandHome(String(configPath).trim());
    }
    this._handle = this._fn.init(pathArg);
    if (!this._handle) throw new Error(this._err('init failed'));
    this._syncAuthIfSet();
    return this;
  }

  connect(baseUrl) {
    const mode = this._ensureHost();
    if (mode === 'native') {
      this._native.connect(baseUrl);
      this._native.setup?.(this._auth.base_url || null, this._auth.token || null);
      return this;
    }
    if (this._handle) this.close();
    this._handle = this._fn.connect(baseUrl);
    if (!this._handle) throw new Error(this._err('connect failed'));
    this._syncAuthIfSet();
    return this;
  }

  close() {
    if (this._native) {
      try {
        this._native.destroy?.() || this._native.close?.();
      } catch {
        /* ignore */
      }
      return;
    }
    if (this._handle && this._fn) this._fn.destroy(this._handle);
    this._handle = null;
  }

  destroy() {
    this.close();
  }

  complete(messages, options = {}) {
    const mode = this._ensureHost();
    if (mode === 'native') {
      const raw = this._native.complete(JSON.stringify(messages), JSON.stringify(options || {}));
      return typeof raw === 'string' ? JSON.parse(raw) : raw;
    }
    if (!this._handle) throw new Error('router not initialized');
    const buf = Buffer.alloc(256 * 1024);
    const rc = this._fn.complete(
      this._handle,
      JSON.stringify(messages),
      JSON.stringify(options || {}),
      buf,
      buf.length,
    );
    if (rc !== 0) throw new Error(this._err('complete failed'));
    return parseJsonBuf(buf);
  }

  models() {
    const mode = this._ensureHost();
    if (mode === 'native') {
      const raw = this._native.models();
      return typeof raw === 'string' ? JSON.parse(raw) : raw;
    }
    if (!this._handle) throw new Error('router not initialized');
    const buf = Buffer.alloc(64 * 1024);
    const rc = this._fn.models(this._handle, buf, buf.length);
    if (rc !== 0) throw new Error(this._err('models failed'));
    return parseJsonBuf(buf);
  }

  lastRoute() {
    const mode = this._ensureHost();
    if (mode === 'native') {
      try {
        const raw = this._native.lastRoute();
        if (!raw) return {};
        return typeof raw === 'string' ? JSON.parse(raw) : raw;
      } catch {
        return {};
      }
    }
    if (!this._handle) return {};
    const buf = Buffer.alloc(64 * 1024);
    const rc = this._fn.lastRoute(this._handle, buf, buf.length);
    if (rc !== 0) return {};
    try {
      return parseJsonBuf(buf);
    } catch {
      return {};
    }
  }
}

module.exports = { defaultSetup, applySetup, Router };
