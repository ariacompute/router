import test from "node:test";
import assert from "node:assert/strict";
import { Router } from "../src/index.js";

const ffi = process.env.ARIA_ROUTER_FFI_LIB;
const cfg = process.env.ARIA_ROUTER_CONFIG;
const hasFfi = Boolean(ffi && cfg);

test("setup memory only", () => {
  const r = new Router();
  r.setup({ base_url: "http://127.0.0.1:8899", token: "t" });
  assert.equal(r.setupStatus().token, "t");
  r.setupClear();
  assert.equal(r.setupStatus().token, "");
});

test("init_ok", { skip: !hasFfi }, () => {
  const r = new Router().init(cfg);
  try {
    const m = r.models();
    assert.ok(JSON.stringify(m).includes("semantic-auto"));
  } finally {
    r.close();
  }
});

test("models_ok", { skip: !hasFfi }, () => {
  const r = new Router().init(cfg);
  try {
    const m = r.models();
    const data = m.data || [];
    assert.ok(data.some((x) => String(x.id || x).includes("semantic-auto")));
  } finally {
    r.close();
  }
});

test("complete_ok", { skip: !hasFfi }, () => {
  const r = new Router().init(cfg);
  try {
    const out = r.complete(
      [{ role: "user", content: "hi" }],
      { model: "ariacompute/semantic-auto" },
    );
    assert.ok(JSON.stringify(out).includes("hello-from-router"));
  } finally {
    r.close();
  }
});

test("init_missing_path", { skip: !hasFfi }, () => {
  assert.throws(() => new Router().init("/no/such.yaml"), /./);
});

test("connect_without_server", { skip: !hasFfi }, () => {
  const r = new Router().connect("http://127.0.0.1:9");
  try {
    assert.ok(r);
  } finally {
    r.close();
  }
});

test("last_route_after_complete", { skip: !hasFfi }, () => {
  const r = new Router().init(cfg);
  try {
    r.complete([{ role: "user", content: "hi" }], { model: "ariacompute/semantic-auto" });
    const lr = r.lastRoute();
    assert.equal(lr.layer, "semantic");
  } finally {
    r.close();
  }
});
