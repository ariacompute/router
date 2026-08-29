import test from "node:test";
import assert from "node:assert/strict";
import { Router } from "../src/index.js";

test("setup memory only", () => {
  const r = new Router();
  r.setup({ base_url: "http://127.0.0.1:8899", token: "t" });
  assert.equal(r.setupStatus().token, "t");
  r.setupClear();
  assert.equal(r.setupStatus().token, "");
});
