import assert from "node:assert/strict";
import {
	mkdirSync,
	mkdtempSync,
	readFileSync,
	symlinkSync,
	writeFileSync,
} from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
	canonicalize,
	grantsToRuntime,
	gitControlRoot,
	isDefaultWritePath,
	isControlRootSymlink,
	isProtectedPath,
	isProtectedWritePath,
	loadWorkspacePermissions,
	mcpPermissionFromInput,
	normalizeNetworkHost,
	normalizePermission,
	permissionCoversPath,
	projectControlRoot,
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
	assert.equal(isProtectedPath(join(homedir(), ".pi", "agent", "auth.json")), true);
	assert.equal(isProtectedPath(join(homedir(), ".codex", "auth.json")), true);
	assert.equal(isProtectedWritePath(join(homedir(), ".pi", "other", "config.json")), true);
	assert.throws(
		() =>
			normalizePermission(
				{ kind: "read", path: join(homedir(), ".pi", "agent", "auth.json") },
				workspace,
			),
		/Protected secret/,
	);
	assert.doesNotThrow(() =>
		normalizePermission(
			{ kind: "read", path: join(homedir(), ".pi", "agent"), targetType: "folder" },
			workspace,
		),
	);
});

test("project Pi paths widen to the project control folder", () => {
	const root = mkdtempSync(join(tmpdir(), "pi-project-control-"));
	const workspace = join(root, "workspace");
	mkdirSync(workspace);
	assert.equal(
		projectControlRoot(join(workspace, ".pi", "extensions", "unsafe.ts"), workspace),
		canonicalize(join(workspace, ".pi")),
	);
	assert.equal(projectControlRoot(join(workspace, "src", "index.ts"), workspace), undefined);
	assert.doesNotThrow(() =>
		normalizePermission(
			{ kind: "write", path: join(workspace, ".pi"), targetType: "folder" },
			workspace,
		),
	);
});

test("git object paths widen only to the repository control folder", () => {
	const root = mkdtempSync(join(tmpdir(), "pi-git-"));
	const workspace = join(root, "workspace");
	const git = join(workspace, ".git");
	mkdirSync(git, { recursive: true });
	assert.equal(gitControlRoot(join(git, "objects", "ab", "new-object")), canonicalize(git));
	assert.equal(gitControlRoot(join(workspace, "src", "index.ts")), undefined);
});

test("a symlinked git folder keeps its lexical control root", () => {
	const root = mkdtempSync(join(tmpdir(), "pi-git-link-"));
	const workspace = join(root, "workspace");
	const target = join(workspace, "git-control");
	const git = join(workspace, ".git");
	mkdirSync(target, { recursive: true });
	symlinkSync(target, git);
	assert.equal(gitControlRoot(join(git, "hooks", "pre-commit"), workspace), git);
	assert.equal(gitControlRoot(join(target, "hooks", "pre-commit"), workspace), git);
	assert.equal(isControlRootSymlink(git), true);
	assert.deepEqual(
		grantsToRuntime([{ kind: "write", path: git, directory: true }]),
		{ read: [], write: [], networkHosts: [] },
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
		networkHosts: [],
	});
	assert.equal(JSON.parse(readFileSync(state, "utf8")).version, 2);
});

test("network rights require one exact normalized host or IP", () => {
	const root = mkdtempSync(join(tmpdir(), "pi-io-"));
	const workspace = join(root, "workspace");
	mkdirSync(workspace);
	const host = normalizePermission({ kind: "network_host", host: "API.Example.COM." }, workspace);
	const ip = normalizePermission({ kind: "network_host", host: "[::1]" }, workspace);
	assert.deepEqual(host, { kind: "network_host", host: "api.example.com" });
	assert.deepEqual(ip, { kind: "network_host", host: "::1" });
	assert.deepEqual(grantsToRuntime([host, ip]), {
		read: [],
		write: [],
		networkHosts: ["::1", "api.example.com"],
	});
	for (const value of [
		"*",
		"*.example.com",
		"https://example.com",
		"example.com:443",
		"example.com/path",
		"[example.com]",
	]) {
		assert.throws(() => normalizeNetworkHost(value), /exact hostname|Invalid/);
	}
});

test("legacy blanket network rights are dropped while narrow rights survive", () => {
	const root = mkdtempSync(join(tmpdir(), "pi-io-"));
	const workspace = join(root, "workspace");
	const state = join(root, "io-permissions.json");
	mkdirSync(workspace);
	writeFileSync(
		state,
		JSON.stringify({
			version: 1,
			workspaces: {
				[canonicalize(workspace)]: [
					{ kind: "web" },
					{ kind: "local_network" },
					{ kind: "network_host", host: "registry.npmjs.org" },
				],
			},
		}),
	);
	assert.deepEqual(loadWorkspacePermissions(state, workspace), [
		{ kind: "network_host", host: "registry.npmjs.org" },
	]);
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
		networkHosts: [],
	});
});
