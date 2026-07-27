import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, symlinkSync } from "node:fs";
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
	assert.equal(isBaseWriteAllowed(join(workspace, ".git", "index"), DEFAULT_CONFIG, workspace), false);
	assert.equal(
		isBaseWriteAllowed(join(workspace, ".pi", "extensions", "unsafe.ts"), DEFAULT_CONFIG, workspace),
		false,
	);
});

test("development caches are writable without granting whole tool homes", () => {
	const workspace = "/work";
	for (const path of [
		join(homedir(), ".cargo", "registry", "cache", "package.crate"),
		join(homedir(), ".cargo", "git", "checkouts", "package", ".git", "index.lock"),
		join(homedir(), ".npm", "_cacache", "entry"),
		join(homedir(), ".bun", "install", "cache", "package"),
		join(homedir(), "go", "pkg", "mod", "cache", "download", "module"),
	]) {
		assert.equal(isBaseWriteAllowed(path, DEFAULT_CONFIG, workspace), true, path);
	}
	for (const path of [
		join(homedir(), ".cargo", "config.toml"),
		join(homedir(), ".cargo", "credentials.toml"),
		join(homedir(), ".cargo", "bin", "cargo-tool"),
		join(homedir(), ".npm-other", "entry"),
	]) {
		assert.equal(isBaseWriteAllowed(path, DEFAULT_CONFIG, workspace), false, path);
	}
});

test("git metadata in a configured cache stays writable", () => {
	assert.equal(
		isBaseWriteAllowed(
			"/cache/cargo/git/checkouts/package/.git/config",
			{ filesystem: { allowWrite: ["/cache"] } },
			"/work",
		),
		true,
	);
});

test("a symlinked workspace git folder stays read-only", () => {
	const root = mkdtempSync(join(tmpdir(), "pi-policy-git-link-"));
	const workspace = join(root, "workspace");
	const target = join(workspace, "git-control");
	mkdirSync(target, { recursive: true });
	symlinkSync(target, join(workspace, ".git"));

	assert.equal(
		isBaseWriteAllowed(join(workspace, ".git", "hooks", "pre-commit"), DEFAULT_CONFIG, workspace),
		false,
	);
	assert.equal(
		isBaseWriteAllowed(join(target, "hooks", "pre-commit"), DEFAULT_CONFIG, workspace),
		false,
	);
});

test("secret file rules cover top-level and nested paths", () => {
	const root = mkdtempSync(join(tmpdir(), "pi-policy-"));
	const workspace = join(root, "workspace");
	mkdirSync(workspace);

	for (const path of [
		join(workspace, ".env"),
		join(workspace, "app", ".env.local"),
		join(workspace, "keys", "deploy.key"),
	]) {
		assert.equal(isDeniedByConfig(canonicalize(path), "read", DEFAULT_CONFIG, workspace), true);
		assert.equal(isDeniedByConfig(canonicalize(path), "write", DEFAULT_CONFIG, workspace), true);
	}
});

test("PEM certificate bundles remain readable but not writable", () => {
	const workspace = canonicalize("/tmp/project");
	const systemBundle = canonicalize("/etc/ssl/cert.pem");

	assert.equal(isDeniedByConfig(systemBundle, "read", DEFAULT_CONFIG, workspace), false);
	assert.equal(isDeniedByConfig(systemBundle, "write", DEFAULT_CONFIG, workspace), true);
});

test("path globs match names without widening sibling prefixes", () => {
	const cwd = canonicalize("/tmp/project");
	assert.equal(matchesPathRule("*.key", canonicalize("/tmp/project/a.key"), cwd), true);
	assert.equal(matchesPathRule("**/.env.*", canonicalize("/tmp/project/app/.env.test"), cwd), true);
	assert.equal(matchesPathRule("/tmp/data", canonicalize("/tmp/database/file"), cwd), false);
});
