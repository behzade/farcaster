import assert from "node:assert/strict";
import { existsSync, mkdtempSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { buildBrokerExecRequest } from "./broker-policy.ts";
import { DEFAULT_CONFIG } from "./codex-command.ts";
import { canonicalize } from "./io-permissions.ts";

test("maps current base rights and command-local folder grants", () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-broker-policy-"));
	const state = join(homedir(), ".local", "share", `issues-fixture-${process.pid}`);
	const request = buildBrokerExecRequest(
		"one",
		"issues search view=issue number=79",
		cwd,
		30,
		DEFAULT_CONFIG,
		[{ kind: "write", path: state, directory: true }],
		[],
	);
	assert.deepEqual(request.command, {
		program: "/bin/bash",
		args: ["-c", "issues search view=issue number=79"],
	});
	assert.equal(request.timeout_ms, 30_000);
	assert.ok(
		request.policy.base_rights.some(
			(right) => right.access === "read" && right.path === "/" && right.scope === "tree",
		),
	);
	assert.ok(
		request.policy.base_rights.some(
			(right) =>
				right.access === "write" &&
				right.path === canonicalize(cwd) &&
				right.scope === "tree",
		),
	);
	const cargoRegistry = canonicalize(join(homedir(), ".cargo", "registry"));
	assert.deepEqual(
		request.policy.base_rights.find((right) => right.path === cargoRegistry),
		{
			access: "write",
			path: cargoRegistry,
			scope: "tree",
			missing_path: existsSync(cargoRegistry) ? "reject" : "create_tree",
		},
	);
	const cargoLock = canonicalize(join(homedir(), ".cargo", ".package-cache"));
	assert.deepEqual(
		request.policy.base_rights.find((right) => right.path === cargoLock),
		{
			access: "write",
			path: cargoLock,
			scope: "file",
			missing_path: existsSync(cargoLock) ? "reject" : "create_file",
		},
	);
	const broadCacheRoots = [
		canonicalize(join(homedir(), ".cargo")),
		canonicalize(join(homedir(), "Library", "Caches")),
	];
	assert.equal(
		request.policy.base_rights.some(
			(right) => right.access === "write" && broadCacheRoots.includes(right.path),
		),
		false,
	);
	assert.deepEqual(request.policy.grants, [
		{
			access: "write",
			path: state,
			scope: "tree",
			missing_path: "create_tree",
		},
	]);
	assert.ok(
		request.policy.denies.some(
			(rule) => rule.access === "read_write" && rule.pattern === "/**/*.key",
		),
	);
});

test("native cache rights overlapping the workspace are omitted", () => {
	const cwd = canonicalize(homedir());
	const request = buildBrokerExecRequest(
		"home-workspace",
		"true",
		cwd,
		undefined,
		DEFAULT_CONFIG,
		[],
		[],
	);
	assert.equal(
		request.policy.base_rights.some(
			(right) => right.path === canonicalize(join(homedir(), ".cargo", "registry")),
		),
		false,
	);
});

test("missing configured read roots are omitted instead of becoming create rights", () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-broker-policy-"));
	const missing = join(cwd, "not-created");
	const request = buildBrokerExecRequest(
		"one",
		"true",
		cwd,
		undefined,
		{
			...DEFAULT_CONFIG,
			filesystem: {
				...DEFAULT_CONFIG.filesystem,
				allowRead: [...(DEFAULT_CONFIG.filesystem?.allowRead ?? []), missing],
			},
		},
		[],
		[],
	);
	assert.equal(request.policy.base_rights.some((right) => right.path === missing), false);
});

test("native deny globs reject dot segments before reaching Rust", () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-broker-policy-"));
	for (const denyWrite of ["dir/../*.secret", "./*.secret", "/tmp/../*.secret"]) {
		assert.throws(
			() =>
				buildBrokerExecRequest(
					"one",
					"true",
					cwd,
					undefined,
					{
						...DEFAULT_CONFIG,
						filesystem: { ...DEFAULT_CONFIG.filesystem, denyWrite: [denyWrite] },
					},
					[],
					[],
				),
			/cannot contain \. or \.\./,
		);
	}
});

test("native preview rejects requested hosts and omits configured socket rights", () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-broker-policy-"));
	assert.throws(
		() => buildBrokerExecRequest("one", "true", cwd, undefined, DEFAULT_CONFIG, [], ["example.com"]),
		/does not yet support network hosts/,
	);
	const request = buildBrokerExecRequest(
		"one",
		"true",
		cwd,
		undefined,
		{
			...DEFAULT_CONFIG,
			network: { ...DEFAULT_CONFIG.network, allowUnixSockets: ["/tmp/service.sock"] },
		},
		[],
		[],
	);
	assert.deepEqual(request.policy.network, { mode: "blocked" });
	assert.equal("unix_socket_roots" in request.policy, false);
});
