import { createRequire } from "node:module";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const __dirname = path.dirname(fileURLToPath(import.meta.url));

function ariaHome() {
  return process.env.ARIA_COMPUTE_HOME || path.join(os.homedir(), ".ariacompute");
}

function ffiLibNames() {
  if (process.platform === "win32") {
    return ["aria-router_ffi.dll", "aria_router_ffi.dll"];
  }
  if (process.platform === "darwin") {
    return ["libaria-router_ffi.dylib", "libaria_router_ffi.dylib"];
  }
  return ["libaria-router_ffi.so", "libaria_router_ffi.so"];
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
  const bundled = firstExisting(path.join(__dirname, "..", "lib"));
  if (bundled) return bundled;
  const cached = firstExisting(path.join(ariaHome(), "lib"));
  if (cached) return cached;
  throw new Error("libaria-router_ffi not found; set ARIA_ROUTER_FFI_LIB");
}

function expandHome(p) {
  if (!p) return p;
  if (p === "~") return os.homedir();
  if (p.startsWith("~/")) return path.join(os.homedir(), p.slice(2));
  return p;
}

function parseJsonBuf(buf) {
  const text = buf.toString("utf8").replace(/\0+$/, "");
  if (!text) return {};
  return JSON.parse(text);
}

export class Router {
  constructor() {
    this._auth = { base_url: "", token: "" };
    this._lib = null;
    this._handle = null;
    this._fn = {};
  }

  setup(u = {}) {
    if (u.base_url !== undefined) this._auth.base_url = u.base_url;
    if (u.token !== undefined) this._auth.token = u.token;
    if (this._handle) {
      this._syncFfiSetup(
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
    this._auth = { base_url: "", token: "" };
    if (this._handle) this._syncFfiSetup("", "");
    return this;
  }

  _ensure(ffiLib) {
    if (this._lib) return;
    const koffi = require("koffi");
    this._lib = koffi.load(resolveLibPath(ffiLib));
    this._fn.init = this._lib.func("aria_router_init", "void*", ["str"]);
    this._fn.connect = this._lib.func("aria_router_connect", "void*", ["str"]);
    this._fn.destroy = this._lib.func("aria_router_destroy", "void", ["void*"]);
    this._fn.setup = this._lib.func("aria_router_setup", "void", ["void*", "str", "str"]);
    this._fn.complete = this._lib.func("aria_router_complete", "int", [
      "void*",
      "str",
      "str",
      "void*",
      "size_t",
    ]);
    this._fn.models = this._lib.func("aria_router_models", "int", ["void*", "void*", "size_t"]);
    this._fn.lastRoute = this._lib.func("aria_router_last_route", "int", [
      "void*",
      "void*",
      "size_t",
    ]);
    this._fn.lastError = this._lib.func("aria_router_last_error", "str", []);
  }

  _syncFfiSetup(baseUrl, token) {
    if (!this._handle || !this._fn.setup) return;
    // koffi: null → NULL pointer (leave unchanged); string updates field.
    this._fn.setup(this._handle, baseUrl, token);
  }

  _syncAuthIfSet() {
    if (this._auth.base_url || this._auth.token) {
      this._syncFfiSetup(
        this._auth.base_url || null,
        this._auth.token || null,
      );
    }
  }

  _err(fallback) {
    const err = this._fn.lastError?.();
    return err || fallback;
  }

  init(configPath, ffiLib) {
    this._ensure(ffiLib);
    if (this._handle) this.close();
    let pathArg = null;
    if (configPath != null && String(configPath).trim() !== "") {
      pathArg = expandHome(String(configPath).trim());
    }
    this._handle = this._fn.init(pathArg);
    if (!this._handle) throw new Error(this._err("init failed"));
    this._syncAuthIfSet();
    return this;
  }

  connect(baseUrl, ffiLib) {
    this._ensure(ffiLib);
    if (this._handle) this.close();
    this._handle = this._fn.connect(baseUrl);
    if (!this._handle) throw new Error(this._err("connect failed"));
    this._syncAuthIfSet();
    return this;
  }

  close() {
    if (this._handle && this._fn.destroy) this._fn.destroy(this._handle);
    this._handle = null;
  }

  destroy() {
    this.close();
  }

  complete(messages, options = {}) {
    if (!this._handle) throw new Error("router not initialized");
    const buf = Buffer.alloc(256 * 1024);
    const rc = this._fn.complete(
      this._handle,
      JSON.stringify(messages),
      JSON.stringify(options || {}),
      buf,
      buf.length,
    );
    if (rc !== 0) throw new Error(this._err("complete failed"));
    return parseJsonBuf(buf);
  }

  models() {
    if (!this._handle) throw new Error("router not initialized");
    const buf = Buffer.alloc(64 * 1024);
    const rc = this._fn.models(this._handle, buf, buf.length);
    if (rc !== 0) throw new Error(this._err("models failed"));
    return parseJsonBuf(buf);
  }

  lastRoute() {
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
