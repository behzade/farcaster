import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const systemPath = new URL("../SYSTEM.md", import.meta.url);
const appendSystemPath = new URL("../APPEND_SYSTEM.md", import.meta.url);
const marker = "<!-- pi:active-tool-guidance -->";

test("static prompt sources stay compact and opt into active tool guidance once", async () => {
  const [systemPrompt, appendSystemPrompt] = await Promise.all([
    readFile(systemPath, "utf8"),
    readFile(appendSystemPath, "utf8"),
  ]);

  assert.ok(systemPrompt.length <= 1_200, `SYSTEM.md is ${systemPrompt.length} chars; limit is 1200`);
  assert.ok(appendSystemPrompt.length <= 3_200, `APPEND_SYSTEM.md is ${appendSystemPrompt.length} chars; limit is 3200`);
  assert.equal(systemPrompt.split(marker).length - 1, 1, "SYSTEM.md must contain exactly one active-guidance marker");
  assert.match(systemPrompt, /Do not run nested `nix develop` commands/);
  assert.match(systemPrompt, /Do not run any Nix command unless the user asks for that exact check/);
  assert.match(appendSystemPrompt, /Never run a Nix command unless the user asks for that exact check/);
  assert.match(systemPrompt, /report the environment defect/);
});
