import assert from "node:assert/strict";
import test from "node:test";
import {
	DEFAULT_CONFIG,
	buildShellEnvironment,
	mergeGlobalConfig,
	normalizeConfig,
} from "./sandbox-config.ts";

test("native preview is the only and default backend", () => {
	assert.equal(DEFAULT_CONFIG.backend, "native-preview");
	assert.equal(normalizeConfig({ backend: "native-preview" }).backend, "native-preview");
	assert.throws(() => normalizeConfig({ backend: "codex" }), /backend must be native-preview/);
	assert.throws(() => normalizeConfig({ codexCommand: "codex" }), /unknown fields/);
	assert.throws(() => normalizeConfig({ permissionProfile: "pi" }), /unknown fields/);
	assert.throws(() => normalizeConfig({ allowPty: true }), /unknown fields/);
});

test("normalizes exact hosts and rejects broad command grants", () => {
	const config = normalizeConfig({
		network: {
			allowedDomains: ["API.Example.COM."],
			deniedDomains: ["*.internal.example"],
			allowUnixSockets: ["/safe.sock"],
		},
	});
	assert.deepEqual(config.network?.allowedDomains, ["api.example.com"]);
	assert.deepEqual(config.network?.deniedDomains, ["*.internal.example"]);
	assert.deepEqual(config.network?.allowUnixSockets, ["/safe.sock"]);
	assert.throws(
		() => normalizeConfig({ network: { allowedDomains: ["*"] } }),
		/exact hostnames or IPs/,
	);
	assert.deepEqual(
		normalizeConfig({ network: { deniedDomains: ["*", "**.Example.COM.", "api.example.com"] } }).network?.deniedDomains,
		["*", "**.example.com", "api.example.com"],
	);
	for (const pattern of ["api.*.example.com", "foo*", "***.example.com", "example.*", "*example.com", "*.127.0.0.1"]) {
		assert.throws(
			() => normalizeConfig({ network: { deniedDomains: [pattern] } }),
			/exact hosts, \*, \*\.domain, or \*\*\.domain/,
			pattern,
		);
	}
});

test("global config extends defaults without dropping hard rules", () => {
	const result = mergeGlobalConfig(
		DEFAULT_CONFIG,
		normalizeConfig({
			filesystem: { allowWrite: ["/state"], denyRead: ["**/private.json"] },
			network: { allowedDomains: ["grafana.example.com"] },
		}),
	);
	assert(result.filesystem?.allowWrite?.includes("."));
	assert(result.filesystem?.allowWrite?.includes("/state"));
	assert(result.filesystem?.denyRead?.includes("~/.ssh"));
	assert(result.filesystem?.denyRead?.includes("**/private.json"));
	assert.deepEqual(result.network?.allowedDomains, ["grafana.example.com"]);
});

test("shell environment preserves the active development shell and removes secret names", () => {
	const environment = buildShellEnvironment(
		DEFAULT_CONFIG,
		{
			PATH: "/bin",
			HOME: "/home/test",
			SDKROOT: "/nix/store/sdk",
			NIX_LDFLAGS: "-L/nix/store/lib",
			BINDGEN_EXTRA_CLANG_ARGS: "-isystem /nix/store/include",
			API_TOKEN: "secret",
			DATABASE_PASSWORD: "secret",
			UNRELATED: "drop",
		},
		"",
	);
	assert.equal(environment.PATH, "/bin");
	assert.equal(environment.HOME, "/home/test");
	assert.equal(environment.SDKROOT, "/nix/store/sdk");
	assert.equal(environment.NIX_LDFLAGS, "-L/nix/store/lib");
	assert.equal(
		environment.BINDGEN_EXTRA_CLANG_ARGS,
		"-isystem /nix/store/include",
	);
	assert.equal(environment.API_TOKEN, undefined);
	assert.equal(environment.DATABASE_PASSWORD, undefined);
	assert.equal(environment.UNRELATED, undefined);
});

test("rejects malformed config instead of weakening policy", () => {
	assert.throws(() => normalizeConfig(null), /JSON object/);
	assert.throws(() => normalizeConfig({ enabled: "yes" }), /enabled/);
	assert.throws(() => normalizeConfig({ brokerPath: "relative" }), /absolute/);
	assert.throws(() => normalizeConfig({ network: { enabled: "yes" } }), /network.enabled/);
	assert.throws(() => normalizeConfig({ network: { allowAllUnixSockets: "yes" } }), /boolean/);
	assert.throws(() => normalizeConfig({ shellEnvironment: { inherit: "some" } }), /inherit/);
});
