import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
	grantsToRuntime,
	isDefaultWritePath,
	loadWorkspacePermissions,
	mcpPermissionFromInput,
	normalizePermission,
	permissionCoversPath,
	saveWorkspacePermission,
} from "./io-permissions.ts";

test("a folder right covers its children but not a sibling", () => {
	const root = mkdtempSync(join(tmpdir(), "pi-io-"));
	const workspace = join(root, "workspace");
	const folder = join(root, "data");
	mkdirSync(workspace);
	mkdirSync(folder);
	const permission = normalizePermission(
		{ kind: "read", path: folder, targetType: "folder" },
		workspace,
	);
	assert.equal(permissionCoversPath(permission, join(folder, "input.csv")), true);
	assert.equal(permissionCoversPath(permission, join(root, "data-other", "input.csv")), false);
});

test("a file right does not widen to its parent folder", () => {
	const root = mkdtempSync(join(tmpdir(), "pi-io-"));
	const workspace = join(root, "workspace");
	const file = join(root, "input.csv");
	mkdirSync(workspace);
	writeFileSync(file, "a,b\n1,2\n");
	const permission = normalizePermission(
		{ kind: "read", path: file, targetType: "file" },
		workspace,
	);
	assert.equal(permissionCoversPath(permission, file), true);
	assert.equal(permissionCoversPath(permission, join(root, "other.csv")), false);
});

test("secret paths cannot be granted", () => {
	const root = mkdtempSync(join(tmpdir(), "pi-io-"));
	const workspace = join(root, "workspace");
	mkdirSync(workspace);
	assert.throws(
		() => normalizePermission({ kind: "read", path: join(root, ".env") }, workspace),
		/Protected secret/,
	);
	assert.throws(
		() =>
			normalizePermission(
				{ kind: "write", path: join(homedir(), ".pi", "agent"), targetType: "folder" },
				workspace,
			),
		/Protected secret or control/,
	);
	assert.doesNotThrow(() =>
		normalizePermission(
			{ kind: "read", path: join(homedir(), ".pi", "agent"), targetType: "folder" },
			workspace,
		),
	);
});

test("workspace and system temp paths are writable by default", () => {
	const root = mkdtempSync(join(tmpdir(), "pi-io-"));
	const workspace = join(root, "workspace");
	mkdirSync(workspace);
	assert.equal(isDefaultWritePath(join(workspace, "result.txt"), workspace), true);
	assert.equal(isDefaultWritePath(join(tmpdir(), "result.txt"), workspace), true);
	assert.equal(
		isDefaultWritePath(join(homedir(), "pi-io-outside", "result.txt"), workspace),
		false,
	);
});

test("always rights stay scoped to the workspace", () => {
	const root = mkdtempSync(join(tmpdir(), "pi-io-"));
	const first = join(root, "first");
	const second = join(root, "second");
	const outside = join(root, "outside");
	const state = join(root, "io-permissions.json");
	mkdirSync(first);
	mkdirSync(second);
	mkdirSync(outside);
	const permission = normalizePermission(
		{ kind: "write", path: outside, targetType: "folder" },
		first,
	);
	saveWorkspacePermission(state, first, permission);
	saveWorkspacePermission(state, first, permission);
	assert.deepEqual(loadWorkspacePermissions(state, first), [permission]);
	assert.deepEqual(loadWorkspacePermissions(state, second), []);
	assert.deepEqual(grantsToRuntime(loadWorkspacePermissions(state, first)), {
		read: [],
		write: [permission.path],
		web: false,
		localNetwork: false,
	});
});

test("one local port request grants localhost once without tracking command text", () => {
	const root = mkdtempSync(join(tmpdir(), "pi-io-"));
	const workspace = join(root, "workspace");
	mkdirSync(workspace);
	const permission = normalizePermission(
		{ kind: "local_port", port: 8317 },
		workspace,
	);
	assert.deepEqual(permission, { kind: "local_network" });
	assert.deepEqual(grantsToRuntime([permission]), {
		read: [],
		write: [],
		web: false,
		localNetwork: true,
	});
});

test("MCP rights use the exact service name and do not change shell grants", () => {
	const permission = mcpPermissionFromInput({ name: "github" });
	assert.deepEqual(permission, { kind: "mcp", server: "github" });
	assert.deepEqual(mcpPermissionFromInput({ serverId: "issues" }), {
		kind: "mcp",
		server: "issues",
	});
	assert.equal(mcpPermissionFromInput({ enabled: true }), undefined);
	assert.deepEqual(grantsToRuntime(permission ? [permission] : []), {
		read: [],
		write: [],
		web: false,
		localNetwork: false,
	});
});
