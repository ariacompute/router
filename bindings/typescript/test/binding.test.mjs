import test from "node:test";
import assert from "node:assert/strict";
import { Router } from "../src/index.js";

test("auth memory only", () => {
  const r = new Router();
  r.auth({ base_url: "http://127.0.0.1:8899", token: "t" });
  assert.equal(r.authStatus().token, "t");
  r.authClear();
  assert.equal(r.authStatus().token, "");
});
