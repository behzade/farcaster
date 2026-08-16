import assert from "node:assert/strict";
import { existsSync, mkdtempSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { buildBrokerExecRequest } from "./broker-policy.ts";
import { DEFAULT_CONFIG } from "./sandbox-config.ts";
import {
	developmentCacheRoot,
	ensureDevelopmentCacheDirectories,
} from "./development-caches.ts";
import { canonicalize } from "./io-permissions.ts";

test("maps current base rights and command-local folder grants", () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-broker-policy-"));
	const canonicalCwd = canonicalize(cwd);
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
	assert.match(request.command.program, /\/bash$/);
	assert.deepEqual(request.command.args, ["-c", "issues search view=issue number=79"]);
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
	const cacheRoot = canonicalize(developmentCacheRoot());
	assert.deepEqual(
		request.policy.base_rights.find((right) => right.path === cacheRoot),
		{
			access: "write",
			path: cacheRoot,
			scope: "tree",
			missing_path: existsSync(cacheRoot) ? "reject" : "create_tree",
		},
	);
	const broadCacheRoots = [
		canonicalize(join(homedir(), ".cargo")),
		canonicalize(join(homedir(), ".npm")),
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
			(rule) =>
				rule.access === "read_write" && rule.pattern === `${canonicalCwd}/**/*.key`,
		),
	);
	assert.ok(
		request.policy.denies.some(
			(rule) =>
				rule.access === "read_write" &&
				rule.pattern === `${canonicalCwd}/**/.env` &&
				rule.scope === "glob",
		),
	);
	assert.equal(
		request.policy.denies.some(
			(rule) => rule.pattern === join(cwd, ".env") && rule.scope !== "glob",
		),
		false,
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
			(right) => right.path === canonicalize(developmentCacheRoot()),
		),
		false,
	);
});

test("native policy honors a configured development cache root", () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-broker-policy-custom-"));
	const config = {
		...DEFAULT_CONFIG,
		developmentCache: { root: ".cache/pi-broker-custom" },
	};
	ensureDevelopmentCacheDirectories(config.developmentCache);
	const customRoot = canonicalize(developmentCacheRoot(config.developmentCache));
	const request = buildBrokerExecRequest(
		"custom-cache",
		"true",
		cwd,
		undefined,
		config,
		[],
		[],
	);

	assert.equal(
		request.policy.base_rights.some(
			(right) => right.access === "write" && right.path === customRoot,
		),
		true,
	);
	assert.equal(
		request.policy.base_rights.some(
			(right) =>
				right.access === "write" &&
				right.path === canonicalize(developmentCacheRoot()),
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

test("native preview maps approved hosts to one command-scoped proxy", () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-broker-policy-"));
	const proxied = buildBrokerExecRequest(
		"one",
		"true",
		cwd,
		undefined,
		DEFAULT_CONFIG,
		[],
		["example.com"],
		{ port: 43127, socketPath: "/tmp/pi-proxy.sock" },
	);
	assert.deepEqual(proxied.policy.network, {
		mode: "proxy",
		tcp_port: 43127,
		unix_socket: "/tmp/pi-proxy.sock",
		allow_local_binding: false,
	});
	const local = buildBrokerExecRequest(
		"local",
		"true",
		cwd,
		undefined,
		DEFAULT_CONFIG,
		[],
		[],
		undefined,
		true,
	);
	assert.deepEqual(local.policy.network, { mode: "loopback" });
	const localAndProxy = buildBrokerExecRequest(
		"local-and-proxy",
		"true",
		cwd,
		undefined,
		DEFAULT_CONFIG,
		[],
		["example.com"],
		{ port: 43127, socketPath: "/tmp/pi-proxy.sock" },
		true,
	);
	assert.deepEqual(localAndProxy.policy.network, {
		mode: "proxy",
		tcp_port: 43127,
		unix_socket: "/tmp/pi-proxy.sock",
		allow_local_binding: true,
	});
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
	assert.deepEqual(request.policy.unix_socket_roots, [canonicalize("/tmp/service.sock")]);
});

test("native preview rejects broad and relative Unix socket access", () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-broker-policy-"));
	assert.throws(
		() =>
			buildBrokerExecRequest(
				"one",
				"true",
				cwd,
				undefined,
				{
					...DEFAULT_CONFIG,
					network: { ...DEFAULT_CONFIG.network, allowAllUnixSockets: true },
				},
				[],
				[],
			),
		/does not support allowing all Unix sockets/,
	);
	assert.throws(
		() =>
			buildBrokerExecRequest(
				"one",
				"true",
				cwd,
				undefined,
				{
					...DEFAULT_CONFIG,
					network: { ...DEFAULT_CONFIG.network, allowUnixSockets: ["service.sock"] },
				},
				[],
				[],
			),
		/must be absolute/,
	);
});
