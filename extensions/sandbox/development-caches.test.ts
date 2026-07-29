import assert from "node:assert/strict";
import {
	existsSync,
	mkdirSync,
	mkdtempSync,
	realpathSync,
	symlinkSync,
	writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, relative, sep } from "node:path";
import test from "node:test";
import {
	developmentCacheEnvironment,
	developmentCacheRightForPath,
	developmentCacheRoot,
	developmentCacheWriteRights,
	developmentCacheWriteRightsForWorkspace,
	ensureDevelopmentCacheDirectories,
} from "./development-caches.ts";

test("development caches use one sandbox-owned writable namespace", () => {
	const home = mkdtempSync(join(tmpdir(), "pi-development-caches-"));
	ensureDevelopmentCacheDirectories(undefined, home);
	const actualHome = realpathSync.native(home);
	const root = join(actualHome, ".cache", "pi-sandbox");

	assert.equal(developmentCacheRoot(undefined, home), root);
	assert.deepEqual(developmentCacheWriteRights(undefined, home), [
		{ path: root, directory: true },
	]);
	const environment = developmentCacheEnvironment(undefined, home);
	assert(Object.keys(environment).length > 0);
	for (const [name, path] of Object.entries(environment)) {
		const rel = relative(root, path);
		assert(rel !== "" && rel !== ".." && !rel.startsWith(`..${sep}`), name);
		assert.equal(existsSync(path), true, name);
		assert.deepEqual(
				developmentCacheRightForPath(path, undefined, home),
			{ path: root, directory: true },
			name,
		);
	}
});

test("unsafe cache roots are neither writable nor exported", () => {
	const linkedHome = mkdtempSync(join(tmpdir(), "pi-development-cache-link-"));
	const outside = mkdtempSync(join(tmpdir(), "pi-development-cache-target-"));
	mkdirSync(join(linkedHome, ".cache"));
	symlinkSync(outside, join(linkedHome, ".cache", "pi-sandbox"));
	ensureDevelopmentCacheDirectories(undefined, linkedHome);
	assert.deepEqual(developmentCacheWriteRights(undefined, linkedHome), []);
	assert.deepEqual(developmentCacheEnvironment(undefined, linkedHome), {});

	const fileHome = mkdtempSync(join(tmpdir(), "pi-development-cache-file-"));
	mkdirSync(join(fileHome, ".cache"));
	writeFileSync(join(fileHome, ".cache", "pi-sandbox"), "not a directory");
	ensureDevelopmentCacheDirectories(undefined, fileHome);
	assert.deepEqual(developmentCacheWriteRights(undefined, fileHome), []);
	assert.deepEqual(developmentCacheEnvironment(undefined, fileHome), {});
});

test("the sandbox cache right is omitted when it overlaps the workspace", () => {
	const home = mkdtempSync(join(tmpdir(), "pi-development-cache-workspace-"));
	ensureDevelopmentCacheDirectories(undefined, home);
	const root = developmentCacheRoot(undefined, home);
	assert.deepEqual(developmentCacheWriteRightsForWorkspace(home, undefined, home), []);
	assert.deepEqual(
		developmentCacheWriteRightsForWorkspace(
			join(root, "cargo", "project"),
			undefined,
			home,
		),
		[],
	);
});

test("cache matching covers only the sandbox-owned namespace", () => {
	const home = mkdtempSync(join(tmpdir(), "pi-development-cache-match-"));
	ensureDevelopmentCacheDirectories(undefined, home);
	const root = developmentCacheRoot(undefined, home);
	assert.deepEqual(
		developmentCacheRightForPath(
			join(root, "cargo", "registry", "package.crate"),
			undefined,
			home,
		),
		{ path: root, directory: true },
	);
	assert.equal(
		developmentCacheRightForPath(
			join(home, ".cargo", ".package-cache"),
			undefined,
			home,
		),
		undefined,
	);
	assert.equal(
		developmentCacheRightForPath(
			join(home, ".cache", "pi-sandbox-other", "entry"),
			undefined,
			home,
		),
		undefined,
	);
});

test("custom cache roots and environment adapters stay within one right", () => {
	const home = mkdtempSync(join(tmpdir(), "pi-development-cache-custom-"));
	const config = {
		root: ".cache/custom-sandbox",
		environment: { CUSTOM_TOOL_CACHE: "custom/tool" },
	};
	ensureDevelopmentCacheDirectories(config, home);
	const root = developmentCacheRoot(config, home);
	const environment = developmentCacheEnvironment(config, home);

	assert.equal(root, join(realpathSync.native(home), ".cache", "custom-sandbox"));
	assert.equal(environment.CUSTOM_TOOL_CACHE, join(root, "custom", "tool"));
	assert.equal(existsSync(environment.CUSTOM_TOOL_CACHE), true);
	assert.deepEqual(developmentCacheWriteRights(config, home), [
		{ path: root, directory: true },
	]);
});
