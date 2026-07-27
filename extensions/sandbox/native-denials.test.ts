import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { DEFAULT_CONFIG } from "./codex-command.ts";
import { permissionForNativeDenial } from "./native-denials.ts";

test("native write denials yield one exact file permission", () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-native-denial-"));
	const path = join(homedir(), `pi-native-denial-${process.pid}`, "state.db");
	assert.deepEqual(
		permissionForNativeDenial(
			{ operation: "file-write-create", path, process: "issues" },
			cwd,
			DEFAULT_CONFIG,
			[],
		),
		{
			kind: "permission",
			permission: { kind: "write", path, directory: false },
		},
	);
});

test("native denial mapping rejects protected and blocked paths", () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-native-denial-"));
	const protectedPath = join(homedir(), ".ssh", "config");
	assert.deepEqual(
		permissionForNativeDenial(
			{ operation: "file-read-data", path: protectedPath, process: "cat" },
			cwd,
			DEFAULT_CONFIG,
			[],
		),
		{ kind: "unsafe" },
	);

	const source = fileURLToPath(import.meta.url);
	const restrictedConfig = {
		...DEFAULT_CONFIG,
		filesystem: {
			...DEFAULT_CONFIG.filesystem,
			allowRead: [],
			allowWrite: [],
		},
	};
	assert.deepEqual(
		permissionForNativeDenial(
			{ operation: "file-read-data", path: source, process: "cat" },
			cwd,
			restrictedConfig,
			[],
			[source],
		),
		{ kind: "unsafe" },
	);
});

test("native ordinary directory denials do not widen to tree grants", () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-native-denial-dir-"));
	const directory = fileURLToPath(new URL(".", import.meta.url));
	const restrictedConfig = {
		...DEFAULT_CONFIG,
		filesystem: {
			...DEFAULT_CONFIG.filesystem,
			allowRead: [],
			allowWrite: [],
		},
	};
	assert.deepEqual(
		permissionForNativeDenial(
			{ operation: "file-read-metadata", path: directory, process: "tool" },
			cwd,
			restrictedConfig,
			[],
		),
		{ kind: "ignore" },
	);
});

test("native git write denials ask for the real control folder", () => {
	const cwd = fileURLToPath(new URL("../..", import.meta.url));
	const git = join(cwd, ".git");
	const path = join(git, "index.lock");
	assert.deepEqual(
		permissionForNativeDenial(
			{ operation: "file-write-create", path, process: "git" },
			cwd,
			DEFAULT_CONFIG,
			[],
		),
		{
			kind: "permission",
			permission: { kind: "write", path: git, directory: true },
		},
	);
});
