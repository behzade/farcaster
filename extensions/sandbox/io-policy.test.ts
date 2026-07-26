import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { DEFAULT_CONFIG } from "./codex-command.ts";
import {
	isBaseReadAllowed,
	isBaseWriteAllowed,
	isDeniedByConfig,
	matchesPathRule,
} from "./io-policy.ts";
import { canonicalize } from "./io-permissions.ts";

test("base rights allow broad reads and only workspace or temp writes", () => {
	const root = mkdtempSync(join(tmpdir(), "pi-policy-"));
	const workspace = join(root, "workspace");
	mkdirSync(workspace);
	const outside = canonicalize(join(homedir(), "pi-policy-outside", "file.txt"));

	assert.equal(isBaseReadAllowed(outside, DEFAULT_CONFIG, workspace), true);
	assert.equal(isBaseWriteAllowed(join(workspace, "out.txt"), DEFAULT_CONFIG, workspace), true);
	assert.equal(isBaseWriteAllowed(join(tmpdir(), "out.txt"), DEFAULT_CONFIG, workspace), true);
	assert.equal(isBaseWriteAllowed(outside, DEFAULT_CONFIG, workspace), false);
});

test("secret file rules cover top-level and nested paths", () => {
	const root = mkdtempSync(join(tmpdir(), "pi-policy-"));
	const workspace = join(root, "workspace");
	mkdirSync(workspace);

	for (const path of [
		join(workspace, ".env"),
		join(workspace, "app", ".env.local"),
		join(workspace, "keys", "deploy.pem"),
	]) {
		assert.equal(isDeniedByConfig(canonicalize(path), "read", DEFAULT_CONFIG, workspace), true);
		assert.equal(isDeniedByConfig(canonicalize(path), "write", DEFAULT_CONFIG, workspace), true);
	}
});

test("path globs match names without widening sibling prefixes", () => {
	const cwd = canonicalize("/tmp/project");
	assert.equal(matchesPathRule("*.key", canonicalize("/tmp/project/a.key"), cwd), true);
	assert.equal(matchesPathRule("**/.env.*", canonicalize("/tmp/project/app/.env.test"), cwd), true);
	assert.equal(matchesPathRule("/tmp/data", canonicalize("/tmp/database/file"), cwd), false);
});
