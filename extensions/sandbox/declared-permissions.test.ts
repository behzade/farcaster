import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { DEFAULT_CONFIG } from "./codex-command.ts";
import { checkDeclaredFilesystemPermissions } from "./declared-permissions.ts";
import { canonicalize } from "./io-permissions.ts";

function workspace(): string {
	return mkdtempSync(join(tmpdir(), "pi-declared-rights-"));
}

test("an external folder write is command-local and needs approval", () => {
	const cwd = workspace();
	const state = `/home/sandbox-user/.local/share/pi-issues-state-${process.pid}`;
	const [checked] = checkDeclaredFilesystemPermissions(
		[{ kind: "write", path: state, targetType: "folder", reason: "write state" }],
		cwd,
		DEFAULT_CONFIG,
		[],
	);
	assert.equal(checked?.permission.path, state);
	assert.equal(checked?.permission.directory, true);
	assert.equal(checked?.alreadyAllowed, false);
	assert.equal(checked?.reason, "write state");
});

test("workspace writes do not prompt again", () => {
	const cwd = workspace();
	const [checked] = checkDeclaredFilesystemPermissions(
		[{ kind: "write", path: join(cwd, "out.txt"), reason: "write output" }],
		cwd,
		DEFAULT_CONFIG,
		[],
	);
	assert.equal(checked?.alreadyAllowed, true);
});

test("declared control writes widen to the full control folder", () => {
	const cwd = workspace();
	mkdirSync(join(cwd, ".git", "hooks"), { recursive: true });
	mkdirSync(join(cwd, ".pi", "extensions"), { recursive: true });
	const checked = checkDeclaredFilesystemPermissions(
		[
			{
				kind: "write",
				path: join(cwd, ".git", "hooks", "pre-commit"),
				reason: "install hook",
			},
			{
				kind: "write",
				path: join(cwd, ".pi", "extensions", "local.ts"),
				reason: "change extension",
			},
		],
		cwd,
		DEFAULT_CONFIG,
		[],
	);
	assert.deepEqual(
		checked.map(({ permission }) => permission),
		[
			{ kind: "write", path: canonicalize(join(cwd, ".git")), directory: true },
			{ kind: "write", path: canonicalize(join(cwd, ".pi")), directory: true },
		],
	);
	assert.equal(checked.every(({ alreadyAllowed }) => !alreadyAllowed), true);
});

test("declared writes cannot grant a symlinked control folder", () => {
	const cwd = workspace();
	const target = join(cwd, "git-control");
	mkdirSync(target);
	symlinkSync(target, join(cwd, ".git"));
	assert.throws(
		() =>
			checkDeclaredFilesystemPermissions(
				[
					{
						kind: "write",
						path: join(cwd, ".git", "hooks", "pre-commit"),
						reason: "install hook",
					},
				],
				cwd,
				DEFAULT_CONFIG,
				[],
			),
		/symlinked control folder/,
	);
});

test("configured denies beat a declared right", () => {
	const cwd = workspace();
	assert.throws(
		() =>
			checkDeclaredFilesystemPermissions(
				[{ kind: "write", path: join(cwd, ".env"), reason: "change env" }],
				cwd,
				DEFAULT_CONFIG,
				[],
			),
		/Protected secret|Sandbox policy denies/,
	);
});

test("the runtime check enforces the declaration cap", () => {
	const cwd = workspace();
	const declarations = Array.from({ length: 17 }, (_, index) => ({
		kind: "write" as const,
		path: join(tmpdir(), `pi-right-${index}`),
		reason: "test",
	}));
	assert.throws(
		() => checkDeclaredFilesystemPermissions(declarations, cwd, DEFAULT_CONFIG, []),
		/at most 16/,
	);
});
