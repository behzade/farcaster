import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, realpathSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
	developmentCacheRightForPath,
	developmentCacheWriteRights,
	developmentCacheWriteRightsForWorkspace,
	ensureDevelopmentCacheDirectories,
} from "./development-caches.ts";

test("development cache defaults use narrow typed paths", () => {
	const home = mkdtempSync(join(tmpdir(), "pi-development-caches-"));
	ensureDevelopmentCacheDirectories(home, "darwin");
	const rights = developmentCacheWriteRights(home, "darwin");
	const byPath = new Map(rights.map((right) => [right.path, right]));
	const actualHome = realpathSync.native(home);

	assert.equal(byPath.get(join(actualHome, ".cargo", "registry"))?.directory, true);
	assert.equal(byPath.get(join(actualHome, ".cargo", ".package-cache"))?.directory, false);
	assert.equal(byPath.get(join(actualHome, ".npm"))?.directory, true);
	assert.equal(byPath.get(join(actualHome, ".cache", "nix"))?.directory, true);
	assert.equal(byPath.get(join(actualHome, ".cache", "uv"))?.directory, true);
	assert.equal(existsSync(join(actualHome, ".bun", "install", "cache")), true);
	assert.equal(byPath.get(join(actualHome, "Library", "pnpm", "store"))?.directory, true);
	assert.equal(byPath.has(join(actualHome, ".cargo")), false);
	assert.equal(byPath.has(join(actualHome, ".bun")), false);
	assert.equal(byPath.has(join(actualHome, "Library", "Caches")), false);
});

test("development cache defaults omit symlinked or type-mismatched roots", () => {
	const home = mkdtempSync(join(tmpdir(), "pi-development-cache-links-"));
	const outside = mkdtempSync(join(tmpdir(), "pi-development-cache-target-"));
	mkdirSync(join(home, ".cargo"), { recursive: true });
	mkdirSync(join(home, ".cargo", ".package-cache"));
	symlinkSync(outside, join(home, ".npm"));
	symlinkSync(outside, join(home, ".bun"));
	ensureDevelopmentCacheDirectories(home, "darwin");

	const rights = developmentCacheWriteRights(home, "darwin");
	const actualHome = realpathSync.native(home);
	assert.equal(rights.some((right) => right.path === join(actualHome, ".npm")), false);
	assert.equal(
		rights.some((right) => right.path === join(actualHome, ".bun", "install", "cache")),
		false,
	);
	assert.equal(
		rights.some((right) => right.path === join(actualHome, ".cargo", ".package-cache")),
		false,
	);
	assert.equal(existsSync(join(outside, "install", "cache")), false);
});

test("development caches overlapping the workspace are omitted", () => {
	const home = mkdtempSync(join(tmpdir(), "pi-development-cache-workspace-"));
	ensureDevelopmentCacheDirectories(home, "darwin");
	assert.deepEqual(developmentCacheWriteRightsForWorkspace(home, home, "darwin"), []);
	assert.equal(
		developmentCacheWriteRightsForWorkspace(
			join(home, ".cargo", "registry", "src", "project"),
			home,
			"darwin",
		).some((right) => right.path === join(realpathSync.native(home), ".cargo", "registry")),
		false,
	);
});

test("development cache matching respects file rights and sibling prefixes", () => {
	const home = mkdtempSync(join(tmpdir(), "pi-development-cache-match-"));
	ensureDevelopmentCacheDirectories(home, "darwin");
	assert.equal(
		developmentCacheRightForPath(
			join(home, ".cargo", "registry", "cache", "package.crate"),
			home,
			"darwin",
		)?.directory,
		true,
	);
	assert.equal(
		developmentCacheRightForPath(
			join(home, ".cache", "nix", "fetcher-cache-v4.sqlite"),
			home,
			"darwin",
		)?.directory,
		true,
	);
	assert.equal(
		developmentCacheRightForPath(join(home, ".cargo", ".package-cache"), home, "darwin")
			?.directory,
		false,
	);
	assert.equal(
		developmentCacheRightForPath(
			join(home, ".cargo", ".package-cache", "child"),
			home,
			"darwin",
		),
		undefined,
	);
	assert.equal(
		developmentCacheRightForPath(join(home, ".npm-other", "entry"), home, "darwin"),
		undefined,
	);
});
