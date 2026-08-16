import assert from "node:assert/strict";
import test from "node:test";
import { runtimeNetworkHosts } from "./network-policy.ts";
import { DEFAULT_CONFIG, mergeGlobalConfig, normalizeConfig } from "./sandbox-config.ts";

const machineConfig = (network: unknown) => mergeGlobalConfig(
	DEFAULT_CONFIG,
	normalizeConfig({ network }),
);

test("disabled machine networking produces an empty runtime host set", () => {
	const config = machineConfig({
		enabled: false,
		allowedDomains: ["machine.example"],
	});
	assert.deepEqual(runtimeNetworkHosts(config, ["project.example"]), []);
});

test("machine domain denies remove conflicting machine and project hosts", () => {
	const config = machineConfig({
		allowedDomains: ["machine.example", "safe.example"],
		deniedDomains: ["machine.example", "*.blocked.example", "**.denied.example"],
	});
	assert.deepEqual(runtimeNetworkHosts(config, [
		"build.blocked.example",
		"denied.example",
		"nested.denied.example",
		"project.example",
	]), ["project.example", "safe.example"]);
});
