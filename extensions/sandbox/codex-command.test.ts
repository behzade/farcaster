import assert from "node:assert/strict";
import test from "node:test";
import {
	applyProjectRestrictions,
	buildCodexSandboxArgs,
	buildShellEnvironment,
	DEFAULT_CONFIG,
	mergeGlobalConfig,
	normalizeConfig,
} from "./codex-command.ts";

function overrides(args: string[]): string[] {
	const sandboxIndex = args.indexOf("sandbox");
	assert.notEqual(sandboxIndex, -1);
	const values: string[] = [];
	for (let index = 0; index < sandboxIndex; index += 2) {
		assert.equal(args[index], "-c");
		values.push(args[index + 1]);
	}
	return values;
}

test("builds a direct codex sandbox command with an explicit profile and cwd", () => {
	const args = buildCodexSandboxArgs("/repo", DEFAULT_CONFIG, "printf '%s' hello");
	const profile = `pi-sandbox-${process.pid}`;
	assert.deepEqual(args.slice(-10), [
		"sandbox",
		"--permission-profile",
		profile,
		"--cd",
		"/repo",
		"--include-managed-config",
		"--",
		"bash",
		"-c",
		"printf '%s' hello",
	]);
	assert.equal(args.includes("linux"), false);
});

test("maps file, domain, and socket policy into Codex profile overrides", () => {
	const args = buildCodexSandboxArgs(
		"/repo",
		{
			permissionProfile: "pi-test",
			filesystem: {
				allowWrite: ["/repo", "/outside"],
				denyWrite: ["/repo/.env", "*.pem"],
				denyRead: ["~/.ssh"],
			},
			network: {
				enabled: true,
				allowedDomains: ["github.com"],
				deniedDomains: ["blocked.example"],
				allowUnixSockets: ["/nix/var/nix/daemon-socket/socket"],
				allowAllUnixSockets: false,
			},
		},
		"true",
	);
	const values = overrides(args);
	const profile = values.find((value) => value.startsWith(`permissions.pi-test-${process.pid}=`));
	assert(profile);
	assert(profile.includes('"extends" = ":workspace"'));
	assert(profile.includes('":root" = "read"'));
	assert(profile.includes('"/outside" = "write"'));
	assert(profile.includes('"/repo/.env" = "read"'));
	assert(profile.includes('"*.pem" = "deny"'));
	assert(profile.includes('"~/.ssh" = "deny"'));
	assert(values.includes("features.network_proxy=true"));
	assert(profile.includes('"github.com" = "allow"'));
	assert(profile.includes('"blocked.example" = "deny"'));
	assert(profile.includes('"/nix/var/nix/daemon-socket/socket" = "allow"'));
	assert(profile.includes('"allow_local_binding" = false'));
});

test("a deny rule outranks a one-shot write grant for the same path", () => {
	const profile = overrides(
		buildCodexSandboxArgs(
			"/repo",
			{
				permissionProfile: "pi-test",
				filesystem: { denyRead: ["/secret"] },
				network: { enabled: false },
			},
			"true",
			{ write: ["/secret"] },
		),
	).find((value) => value.startsWith(`permissions.pi-test-${process.pid}=`));
	assert(profile);
	assert(profile.includes('"/secret" = "deny"'));
	assert.equal(profile.includes('"/secret" = "write"'), false);
});

test("a trusted project can only tighten global policy", () => {
	const global = mergeGlobalConfig(DEFAULT_CONFIG, {
		network: {
			enabled: true,
			allowedDomains: ["github.com"],
			allowLocalNetwork: true,
			allowUnixSockets: ["/safe.sock"],
		},
		filesystem: { allowWrite: [".", "/global"] },
	});
	const result = applyProjectRestrictions(global, {
		enabled: false,
		codexCommand: "other-codex",
		permissionProfile: "other",
		network: {
			enabled: false,
			allowedDomains: ["evil.example"],
			allowLocalNetwork: false,
			allowUnixSockets: ["/evil.sock"],
			allowAllUnixSockets: true,
			deniedDomains: ["blocked.example"],
		},
		filesystem: {
			allowWrite: ["/evil"],
			denyRead: ["/secret"],
		},
	});
	assert.equal(result.enabled, true);
	assert.equal(result.codexCommand, "codex");
	assert.equal(result.permissionProfile, "pi-sandbox");
	assert.equal(result.network?.enabled, false);
	assert.deepEqual(result.network?.allowedDomains, ["github.com"]);
	assert.equal(result.network?.allowLocalNetwork, false);
	assert.deepEqual(result.network?.allowUnixSockets, ["/safe.sock"]);
	assert.equal(result.network?.allowAllUnixSockets, false);
	assert.deepEqual(result.network?.deniedDomains, ["blocked.example"]);
	assert.deepEqual(result.filesystem?.allowWrite, [
		".",
		":tmpdir",
		":slash_tmp",
		"/global",
	]);
	assert(result.filesystem?.denyWrite?.includes("~/.pi/agent"));
	assert(result.filesystem?.denyWrite?.includes("~/.codex"));
	assert.deepEqual(result.filesystem?.denyRead, [
		"~/.ssh",
		"~/.aws",
		"~/.gnupg",
		"/**/.env",
		"/**/.env.*",
		"/**/*.key",
		"/secret",
	]);
});

test("an omitted global domain list keeps the safe defaults", () => {
	const global = mergeGlobalConfig(
		DEFAULT_CONFIG,
		normalizeConfig({ network: { allowAllUnixSockets: true } }),
	);
	assert.deepEqual(global.network?.allowedDomains, DEFAULT_CONFIG.network?.allowedDomains);
	assert.equal(global.network?.allowAllUnixSockets, true);
});

test("default network policy has no external domains", () => {
	assert.deepEqual(DEFAULT_CONFIG.network?.allowedDomains, []);
	const profile = overrides(
		buildCodexSandboxArgs("/repo", DEFAULT_CONFIG, "true"),
	).find((value) => value.startsWith(`permissions.pi-sandbox-${process.pid}=`));
	assert(profile);
	assert(profile.includes('"domains" = {  }'));
	assert.equal(profile.includes('"github.com" = "allow"'), false);
	assert.equal(profile.includes('"registry.npmjs.org" = "allow"'), false);
});

test("shell environment defaults to Codex core variables and removes secret names", () => {
	const environment = buildShellEnvironment(DEFAULT_CONFIG, {
		PATH: "/bin",
		HOME: "/home/test",
		TMPDIR: "/tmp/test",
		LANG: "en_US.UTF-8",
		SHLVL: "2",
		ANTHROPIC_AUTH_TOKEN: "secret",
		MY_SECRET_FLAG: "secret",
		XDG_CONFIG_HOME: "/home/test/.config",
	});
	assert.deepEqual(environment, {
		PATH: "/bin",
		HOME: "/home/test",
		TMPDIR: "/tmp/test",
		LANG: "en_US.UTF-8",
		SHLVL: "2",
	});
});

test("shell environment applies excludes, set values, then include-only filters", () => {
	const environment = buildShellEnvironment(
		{
			shellEnvironment: {
				inherit: "all",
				ignoreDefaultExcludes: true,
				exclude: ["AWS_*"],
				set: { SAFE_FLAG: "set", DROP_ME: "set" },
				includeOnly: ["PATH", "SAFE_*", "*TOKEN"],
			},
		},
		{
			PATH: "/bin",
			AWS_PROFILE: "prod",
			GH_TOKEN: "secret",
			OTHER: "drop",
		},
	);
	assert.deepEqual(environment, {
		PATH: "/bin",
		GH_TOKEN: "secret",
		SAFE_FLAG: "set",
	});
});

test("rejects malformed config instead of weakening policy", () => {
	assert.throws(() => normalizeConfig({ network: { enabled: "yes" } }), /network.enabled/);
	assert.throws(
		() => normalizeConfig({ network: { allowLocalNetwork: "yes" } }),
		/network.allowLocalNetwork/,
	);
	assert.throws(() => normalizeConfig({ filesystem: { denyRead: [""] } }), /non-empty strings/);
	assert.throws(() => normalizeConfig({ permissionProfile: 'bad" -c sandbox_mode' }), /permissionProfile/);
	assert.throws(() => normalizeConfig({ filesystem: { allowWrites: ["."] } }), /unknown fields/);
	assert.throws(
		() => normalizeConfig({ shellEnvironment: { inherit: "everything" } }),
		/shellEnvironment.inherit/,
	);
	assert.throws(
		() => normalizeConfig({ shellEnvironment: { set: { "BAD-NAME": "value" } } }),
		/valid environment names/,
	);
	assert.throws(
		() => normalizeConfig({ network: { allowedDomains: ["localhost:8317"] } }),
		/without schemes, paths, or ports/,
	);
});

test("a web grant opens public hosts while configured denies stay in force", () => {
	const profile = overrides(
		buildCodexSandboxArgs(
			"/repo",
			{
				permissionProfile: "pi-test",
				filesystem: { allowRead: [":root"], allowWrite: ["."] },
				network: {
					enabled: true,
					allowedDomains: ["github.com"],
					deniedDomains: ["blocked.example"],
				},
			},
			"true",
			{ web: true },
		),
	).find((value) => value.startsWith(`permissions.pi-test-${process.pid}=`));
	assert(profile);
	assert(profile.includes('"*" = "allow"'));
	assert(profile.includes('"blocked.example" = "deny"'));
});

test("a local network grant enables Codex local binding and loopback hosts", () => {
	const profile = overrides(
		buildCodexSandboxArgs(
			"/repo",
			{
				permissionProfile: "pi-test",
				network: {
					enabled: true,
					allowedDomains: ["github.com"],
				},
			},
			"true",
			{ localNetwork: true },
		),
	).find((value) => value.startsWith(`permissions.pi-test-${process.pid}=`));
	assert(profile);
	assert(profile.includes('"allow_local_binding" = true'));
	assert(profile.includes('"localhost" = "allow"'));
	assert(profile.includes('"127.0.0.1" = "allow"'));
	assert(profile.includes('"::1" = "allow"'));
});

test("a trusted project can only tighten the shell environment", () => {
	const base = mergeGlobalConfig(DEFAULT_CONFIG, {
		shellEnvironment: {
			inherit: "core",
			ignoreDefaultExcludes: false,
			exclude: ["AWS_*"],
			set: { HOST_FLAG: "1" },
		},
	});
	const result = applyProjectRestrictions(base, {
		shellEnvironment: {
			inherit: "all",
			ignoreDefaultExcludes: true,
			exclude: ["AZURE_*"],
			set: { PROJECT_FLAG: "1" },
			includeOnly: ["PATH", "HOME"],
		},
	});
	assert.equal(result.shellEnvironment?.inherit, "core");
	assert.equal(result.shellEnvironment?.ignoreDefaultExcludes, false);
	assert.deepEqual(result.shellEnvironment?.exclude, ["AWS_*", "AZURE_*"]);
	assert.deepEqual(result.shellEnvironment?.set, { HOST_FLAG: "1" });
	assert.deepEqual(result.shellEnvironment?.includeOnly, ["PATH", "HOME"]);
});
