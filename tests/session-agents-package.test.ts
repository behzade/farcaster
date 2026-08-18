import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("session agents package installs the local completion-driven Effect extension", async () => {
	const [expression, index, adapter, packageJson] = await Promise.all([
		readFile(new URL("../nix/pi-session-agents.nix", import.meta.url), "utf8"),
		readFile(new URL("../extensions/subagents/index.ts", import.meta.url), "utf8"),
		readFile(new URL("../extensions/subagents/adapter.ts", import.meta.url), "utf8"),
		readFile(new URL("../extensions/subagents/package.json", import.meta.url), "utf8"),
	]);
	assert.match(expression, /source = \.\.\/extensions\/subagents/);
	assert.doesNotMatch(expression, /fetchFromGitHub|config\.json/);
	assert.deepEqual(
		[...index.matchAll(/name: "(subagent_[a-z]+)"/g)].map((match) => match[1]),
		["subagent_start", "subagent_send", "subagent_control"],
	);
	assert.match(adapter, /await session\.bindExtensions\(\{ mode: "print" \}\)/);
	assert.match(adapter, /dispose: \(\) => runtime\.dispose\(\)/);
	assert.match(
		adapter,
		/excludeTools: \["subagent_start", "subagent_send", "subagent_control"\]/,
	);
	assert.match(adapter, /forkBeforeActiveToolCall/);
	assert.match(adapter, /manager\.branch\(leaf\.parentId\)/);
	assert.match(index, /customType: "subagent-result"/);
	assert.match(index, /triggerTurn: true, deliverAs: "steer"/);
	assert.doesNotMatch(index, /subagent_wait/);
	assert.equal(JSON.parse(packageJson).dependencies.effect, "4.0.0-beta.107");
});
