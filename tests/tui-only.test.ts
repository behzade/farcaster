import assert from "node:assert/strict";
import test from "node:test";
import { registerOnceForTui } from "../extensions/dense-tools/tui-only.ts";

test("terminal renderers register once in TUI and never in headless modes", () => {
  let registrations = 0;
  const register = registerOnceForTui(() => {
    registrations += 1;
  });

  assert.equal(register("rpc"), false);
  assert.equal(register("json"), false);
  assert.equal(register("print"), false);
  assert.equal(registrations, 0);
  assert.equal(register("tui"), true);
  assert.equal(register("tui"), false);
  assert.equal(registrations, 1);
});
