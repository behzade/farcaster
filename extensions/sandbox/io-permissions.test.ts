import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
	canonicalize,
	gitControlRoot,
	isControlRootSymlink,
	isInside,
	isProtectedPath,
	normalizeNetworkHost,
	permissionCoversPath,
	projectControlRoot,
	resolvePermissionPath,
} from "./io-permissions.ts";

test("portable path resolution and file/tree coverage use canonical paths", () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-path-policy-"));
	mkdirSync(join(cwd, "tree"));
	writeFileSync(join(cwd, "tree", "file"), "x");
	assert.equal(resolvePermissionPath("tree", cwd), canonicalize(join(cwd, "tree")));
	assert(permissionCoversPath({ kind: "read", path: join(cwd, "tree"), directory: true }, join(cwd, "tree", "file")));
	assert(!permissionCoversPath({ kind: "read", path: join(cwd, "tree"), directory: false }, join(cwd, "tree", "file")));
	assert(isInside(cwd, join(cwd, "tree", "file")));
});

test("project control roots are identified before symlink canonicalization", () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-control-roots-"));
	mkdirSync(join(cwd, ".git"));
	mkdirSync(join(cwd, ".pi"));
	assert.equal(gitControlRoot(join(cwd, ".git", "index"), cwd), join(cwd, ".git"));
	assert.equal(projectControlRoot(join(cwd, ".pi", "extensions", "sandbox", "sandbox.json"), cwd), join(cwd, ".pi"));
	const target = mkdtempSync(join(tmpdir(), "pi-git-target-"));
	const linked = mkdtempSync(join(tmpdir(), "pi-git-link-"));
	symlinkSync(target, join(linked, ".git"));
	assert.equal(isControlRootSymlink(join(linked, ".git")), true);
});

test("hard protected paths and exact network hosts remain strict", () => {
	assert.equal(isProtectedPath("/dev/tty"), true);
	assert.equal(normalizeNetworkHost("API.Example.COM."), "api.example.com");
	assert.equal(normalizeNetworkHost("[::1]"), "::1");
	assert.throws(() => normalizeNetworkHost("https://example.com"), /exact hostname/);
	assert.throws(() => normalizeNetworkHost("*.example.com"), /exact hostname/);
});
