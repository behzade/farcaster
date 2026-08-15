import assert from "node:assert/strict";
import test from "node:test";
import {
	DEFAULT_CONFIG,
	applyProjectRestrictions,
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

test("a trusted project can only tighten global policy", () => {
	const base = mergeGlobalConfig(
		DEFAULT_CONFIG,
		normalizeConfig({
			network: { allowedDomains: ["grafana.example.com"] },
			filesystem: { allowWrite: ["/state"] },
			shellEnvironment: { inherit: "all", set: { SAFE_VALUE: "yes" } },
		}),
	);
	const result = applyProjectRestrictions(
		base,
		normalizeConfig({
			enabled: false,
			network: { enabled: false, allowedDomains: ["evil.example"] },
			filesystem: { allowWrite: ["/other"], denyWrite: ["**/*.lock"] },
			shellEnvironment: { inherit: "none", set: { INJECTED: "no" } },
		}),
	);
	assert.equal(result.enabled, true);
	assert.equal(result.network?.enabled, false);
	assert.deepEqual(result.network?.allowedDomains, ["grafana.example.com"]);
	assert(result.filesystem?.allowWrite?.includes("/state"));
	assert(!result.filesystem?.allowWrite?.includes("/other"));
	assert(result.filesystem?.denyWrite?.includes("**/*.lock"));
	assert.equal(result.shellEnvironment?.inherit, "none");
	assert.deepEqual(result.shellEnvironment?.set, { SAFE_VALUE: "yes" });
});

test("shell environment keeps core values and removes secret names", () => {
	const environment = buildShellEnvironment(
		DEFAULT_CONFIG,
		{
			PATH: "/bin",
			HOME: "/home/test",
			LANG: "en_US.UTF-8",
			API_TOKEN: "secret",
			UNRELATED: "drop",
		},
		"",
	);
	assert.equal(environment.PATH, "/bin");
	assert.equal(environment.HOME, "/home/test");
	assert.equal(environment.API_TOKEN, undefined);
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
