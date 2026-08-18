import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("session agents package installs the local four-tool Effect extension", async () => {
	const [expression, index, packageJson] = await Promise.all([
		readFile(new URL("../nix/pi-session-agents.nix", import.meta.url), "utf8"),
		readFile(new URL("../extensions/subagents/index.ts", import.meta.url), "utf8"),
		readFile(new URL("../extensions/subagents/package.json", import.meta.url), "utf8"),
	]);
	assert.match(expression, /source = \.\.\/extensions\/subagents/);
	assert.doesNotMatch(expression, /fetchFromGitHub|config\.json/);
	assert.deepEqual(
		[...index.matchAll(/name: "(subagent_[a-z]+)"/g)].map((match) => match[1]),
		["subagent_start", "subagent_send", "subagent_wait", "subagent_control"],
	);
	assert.equal(JSON.parse(packageJson).dependencies.effect, "4.0.0-beta.107");
});
