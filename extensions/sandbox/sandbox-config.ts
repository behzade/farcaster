import { dirname } from "node:path";
import {
	DEFAULT_DEVELOPMENT_CACHE_CONFIG,
	developmentCacheEnvironment,
	type DevelopmentCacheConfig,
	normalizeDevelopmentCacheConfig,
} from "./development-caches.ts";
import { normalizeNetworkHost } from "./io-permissions.ts";

const PACKAGED_MCP_CLI = "@PI_MCP_CLI@";

export interface NativeSandboxNetworkConfig {
	enabled?: boolean;
	allowedDomains?: string[];
	deniedDomains?: string[];
	allowUnixSockets?: string[];
	allowAllUnixSockets?: boolean;
}

export interface NativeSandboxFilesystemConfig {
	allowRead?: string[];
	denyRead?: string[];
	allowWrite?: string[];
	denyWrite?: string[];
}

export type ShellEnvironmentInheritance = "all" | "core" | "none";

export interface NativeSandboxShellEnvironmentConfig {
	inherit?: ShellEnvironmentInheritance;
	ignoreDefaultExcludes?: boolean;
	exclude?: string[];
	includeOnly?: string[];
	set?: Record<string, string>;
}

export interface NativeSandboxGrants {
	read?: readonly string[];
	write?: readonly string[];
	networkHosts?: readonly string[];
}

export interface NativeSandboxConfig {
	enabled?: boolean;
	backend?: "native-preview";
	brokerPath?: string;
	developmentCache?: DevelopmentCacheConfig;
	network?: NativeSandboxNetworkConfig;
	filesystem?: NativeSandboxFilesystemConfig;
	shellEnvironment?: NativeSandboxShellEnvironmentConfig;
}

export const DEFAULT_CONFIG: Required<
	Pick<NativeSandboxConfig, "enabled" | "backend">
> &
	NativeSandboxConfig = {
	enabled: true,
	backend: "native-preview",
	developmentCache: DEFAULT_DEVELOPMENT_CACHE_CONFIG,
	network: {
		enabled: true,
		allowedDomains: [],
		deniedDomains: [],
		allowUnixSockets: [],
		allowAllUnixSockets: false,
	},
	filesystem: {
		allowRead: [":root"],
		denyRead: [
			"~/.ssh",
			"~/.aws",
			"~/.gnupg",
			"~/.pi/agent/auth.json",
			"~/.codex/auth.json",
			"**/.env",
			"**/.env.*",
			"**/*.key",
		],
		allowWrite: [".", ":tmpdir", ":slash_tmp"],
		denyWrite: [
			"**/.env",
			"**/.env.*",
			"**/*.pem",
			"**/*.key",
			"~/.pi",
			"~/.codex",
		],
	},
	shellEnvironment: {
		inherit: "core",
		ignoreDefaultExcludes: false,
		exclude: [],
		includeOnly: [],
		set: {},
	},
};

const ENV_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;
const UNIX_CORE_ENV_VARS = [
	"PATH",
	"SHELL",
	"TMPDIR",
	"TEMP",
	"TMP",
	"HOME",
	"LANG",
	"LC_ALL",
	"LC_CTYPE",
	"LOGNAME",
	"USER",
	"SHLVL",
] as const;

function unique(values: readonly string[]): string[] {
	return [...new Set(values)];
}

function nonEmptyStrings(value: unknown, field: string): string[] | undefined {
	if (value === undefined) return undefined;
	if (!Array.isArray(value) || value.some((entry) => typeof entry !== "string" || entry.length === 0)) {
		throw new Error(`${field} must contain only non-empty strings`);
	}
	return unique(value);
}

function domainPatterns(value: unknown, field: string): string[] | undefined {
	const entries = nonEmptyStrings(value, field);
	if (
		entries?.some(
			(entry) =>
				entry !== "*" &&
				(entry.includes("://") ||
					/:\d+$/.test(entry) ||
					entry.startsWith("[") ||
					entry.includes("/")),
		)
	) {
		throw new Error(`${field} accepts host patterns without schemes, paths, or ports`);
	}
	return entries;
}

function exactNetworkHosts(value: unknown, field: string): string[] | undefined {
	const entries = nonEmptyStrings(value, field);
	if (!entries) return undefined;
	try {
		return unique(entries.map(normalizeNetworkHost));
	} catch (error) {
		throw new Error(
			`${field} must contain exact hostnames or IPs: ${
				error instanceof Error ? error.message : error
			}`,
		);
	}
}

function assertKnownKeys(value: Record<string, unknown>, allowed: readonly string[], field: string): void {
	const unknown = Object.keys(value).filter((key) => !allowed.includes(key));
	if (unknown.length > 0) throw new Error(`${field} contains unknown fields: ${unknown.join(", ")}`);
}

function stringMap(value: unknown, field: string): Record<string, string> | undefined {
	if (value === undefined) return undefined;
	if (!value || typeof value !== "object" || Array.isArray(value)) {
		throw new Error(`${field} must be a JSON object`);
	}
	const entries = Object.entries(value);
	if (
		entries.some(
			([name, entry]) => !ENV_NAME.test(name) || typeof entry !== "string",
		)
	) {
		throw new Error(`${field} must map valid environment names to strings`);
	}
	return Object.fromEntries(entries);
}

export function normalizeConfig(value: unknown): NativeSandboxConfig {
	if (!value || typeof value !== "object" || Array.isArray(value)) {
		throw new Error("sandbox config must be a JSON object");
	}
	const input = value as Record<string, unknown>;
	assertKnownKeys(
		input,
		[
			"enabled",
			"backend",
			"brokerPath",
			"developmentCache",
			"network",
			"filesystem",
			"shellEnvironment",
		],
		"sandbox config",
	);
	const enabled = input.enabled;
	const backend = input.backend;
	const brokerPath = input.brokerPath;
	if (enabled !== undefined && typeof enabled !== "boolean") {
		throw new Error("enabled must be a boolean");
	}
	if (backend !== undefined && backend !== "native-preview") {
		throw new Error("backend must be native-preview");
	}
	if (
		brokerPath !== undefined &&
		(typeof brokerPath !== "string" || !brokerPath.startsWith("/"))
	) {
		throw new Error("brokerPath must be an absolute path");
	}

	const networkInput =
		input.network === undefined
			? undefined
			: input.network && typeof input.network === "object" && !Array.isArray(input.network)
				? (input.network as Record<string, unknown>)
				: (() => {
						throw new Error("network must be a JSON object");
					})();
	if (networkInput?.enabled !== undefined && typeof networkInput.enabled !== "boolean") {
		throw new Error("network.enabled must be a boolean");
	}
	if (
		networkInput?.allowAllUnixSockets !== undefined &&
		typeof networkInput.allowAllUnixSockets !== "boolean"
	) {
		throw new Error("network.allowAllUnixSockets must be a boolean");
	}
	if (networkInput) {
		assertKnownKeys(
			networkInput,
			[
				"enabled",
				"allowedDomains",
				"deniedDomains",
				"allowUnixSockets",
				"allowAllUnixSockets",
			],
			"network",
		);
	}

	const filesystemInput =
		input.filesystem === undefined
			? undefined
			: input.filesystem && typeof input.filesystem === "object" && !Array.isArray(input.filesystem)
				? (input.filesystem as Record<string, unknown>)
				: (() => {
					throw new Error("filesystem must be a JSON object");
				})();
	if (filesystemInput) {
		assertKnownKeys(
			filesystemInput,
			["allowRead", "denyRead", "allowWrite", "denyWrite"],
			"filesystem",
		);
	}

	const shellEnvironmentInput =
		input.shellEnvironment === undefined
			? undefined
			: input.shellEnvironment &&
				  typeof input.shellEnvironment === "object" &&
				  !Array.isArray(input.shellEnvironment)
				? (input.shellEnvironment as Record<string, unknown>)
				: (() => {
						throw new Error("shellEnvironment must be a JSON object");
					})();
	if (shellEnvironmentInput) {
		assertKnownKeys(
			shellEnvironmentInput,
			["inherit", "ignoreDefaultExcludes", "exclude", "includeOnly", "set"],
			"shellEnvironment",
		);
		if (
			shellEnvironmentInput.inherit !== undefined &&
			!["all", "core", "none"].includes(shellEnvironmentInput.inherit as string)
		) {
			throw new Error("shellEnvironment.inherit must be all, core, or none");
		}
		if (
			shellEnvironmentInput.ignoreDefaultExcludes !== undefined &&
			typeof shellEnvironmentInput.ignoreDefaultExcludes !== "boolean"
		) {
			throw new Error("shellEnvironment.ignoreDefaultExcludes must be a boolean");
		}
	}

	return {
		enabled: enabled as boolean | undefined,
		backend: backend as "native-preview" | undefined,
		brokerPath: brokerPath as string | undefined,
		developmentCache: normalizeDevelopmentCacheConfig(input.developmentCache),
		network: networkInput
			? {
					enabled: networkInput.enabled as boolean | undefined,
					allowedDomains: exactNetworkHosts(
						networkInput.allowedDomains,
						"network.allowedDomains",
					),
					deniedDomains: domainPatterns(networkInput.deniedDomains, "network.deniedDomains"),
					allowUnixSockets: nonEmptyStrings(networkInput.allowUnixSockets, "network.allowUnixSockets"),
					allowAllUnixSockets: networkInput.allowAllUnixSockets as boolean | undefined,
				}
			: undefined,
		filesystem: filesystemInput
			? {
					allowRead: nonEmptyStrings(filesystemInput.allowRead, "filesystem.allowRead"),
					denyRead: nonEmptyStrings(filesystemInput.denyRead, "filesystem.denyRead"),
					allowWrite: nonEmptyStrings(filesystemInput.allowWrite, "filesystem.allowWrite"),
					denyWrite: nonEmptyStrings(filesystemInput.denyWrite, "filesystem.denyWrite"),
				}
			: undefined,
		shellEnvironment: shellEnvironmentInput
			? {
					inherit: shellEnvironmentInput.inherit as
						| ShellEnvironmentInheritance
						| undefined,
					ignoreDefaultExcludes: shellEnvironmentInput.ignoreDefaultExcludes as
						| boolean
						| undefined,
					exclude: nonEmptyStrings(
						shellEnvironmentInput.exclude,
						"shellEnvironment.exclude",
					),
					includeOnly: nonEmptyStrings(
						shellEnvironmentInput.includeOnly,
						"shellEnvironment.includeOnly",
					),
					set: stringMap(shellEnvironmentInput.set, "shellEnvironment.set"),
				}
			: undefined,
	};
}

export function mergeGlobalConfig(
	defaults: NativeSandboxConfig,
	override: NativeSandboxConfig,
): NativeSandboxConfig {
	const defined = <T extends object>(value: T | undefined): Partial<T> =>
		Object.fromEntries(
			Object.entries(value ?? {}).filter(([, entry]) => entry !== undefined),
		) as Partial<T>;
	return {
		...defaults,
		...defined(override),
		developmentCache: {
			...defaults.developmentCache,
			...defined(override.developmentCache),
			environment: {
				...(defaults.developmentCache?.environment ?? {}),
				...(override.developmentCache?.environment ?? {}),
			},
		},
		network: { ...defaults.network, ...defined(override.network) },
		filesystem: {
			...defaults.filesystem,
			...defined(override.filesystem),
			allowRead: unique([
				...(defaults.filesystem?.allowRead ?? []),
				...(override.filesystem?.allowRead ?? []),
			]),
			allowWrite: unique([
				...(defaults.filesystem?.allowWrite ?? []),
				...(override.filesystem?.allowWrite ?? []),
			]),
			denyRead: unique([
				...(defaults.filesystem?.denyRead ?? []),
				...(override.filesystem?.denyRead ?? []),
			]),
			denyWrite: unique([
				...(defaults.filesystem?.denyWrite ?? []),
				...(override.filesystem?.denyWrite ?? []),
			]),
		},
		shellEnvironment: {
			...defaults.shellEnvironment,
			...defined(override.shellEnvironment),
			exclude: unique([
				...(defaults.shellEnvironment?.exclude ?? []),
				...(override.shellEnvironment?.exclude ?? []),
			]),
			set: {
				...(defaults.shellEnvironment?.set ?? {}),
				...(override.shellEnvironment?.set ?? {}),
			},
		},
	};
}

function stricterInheritance(
	base: ShellEnvironmentInheritance | undefined,
	project: ShellEnvironmentInheritance | undefined,
): ShellEnvironmentInheritance | undefined {
	const rank: Record<ShellEnvironmentInheritance, number> = {
		all: 0,
		core: 1,
		none: 2,
	};
	if (!base) return project;
	if (!project) return base;
	return rank[project] > rank[base] ? project : base;
}

export function applyProjectRestrictions(
	base: NativeSandboxConfig,
	project: NativeSandboxConfig,
): NativeSandboxConfig {
	return {
		...base,
		// A project file may tighten the active profile, but it may not turn off
		// the host sandbox or add rights.
		enabled: base.enabled,
		// A project cannot relocate the cache, add environment values, or
		// otherwise expand the implicit write namespace.
		developmentCache: base.developmentCache,
		network: {
			...base.network,
			enabled:
				base.network?.enabled === false || project.network?.enabled === false
					? false
					: base.network?.enabled,
				allowedDomains: base.network?.allowedDomains,
				deniedDomains: unique([
				...(base.network?.deniedDomains ?? []),
				...(project.network?.deniedDomains ?? []),
			]),
			allowUnixSockets: base.network?.allowUnixSockets,
			allowAllUnixSockets: base.network?.allowAllUnixSockets,
		},
		filesystem: {
			...base.filesystem,
			allowRead: base.filesystem?.allowRead,
			allowWrite: base.filesystem?.allowWrite,
			denyRead: unique([
				...(base.filesystem?.denyRead ?? []),
				...(project.filesystem?.denyRead ?? []),
			]),
			denyWrite: unique([
				...(base.filesystem?.denyWrite ?? []),
				...(project.filesystem?.denyWrite ?? []),
			]),
		},
		shellEnvironment: {
			...base.shellEnvironment,
			inherit: stricterInheritance(
				base.shellEnvironment?.inherit,
				project.shellEnvironment?.inherit,
			),
			ignoreDefaultExcludes:
				base.shellEnvironment?.ignoreDefaultExcludes === false ||
				project.shellEnvironment?.ignoreDefaultExcludes === false
					? false
					: base.shellEnvironment?.ignoreDefaultExcludes,
			exclude: unique([
				...(base.shellEnvironment?.exclude ?? []),
				...(project.shellEnvironment?.exclude ?? []),
			]),
			includeOnly:
				(base.shellEnvironment?.includeOnly?.length ?? 0) > 0
					? base.shellEnvironment?.includeOnly
					: project.shellEnvironment?.includeOnly,
			// A project cannot inject environment values.
			set: base.shellEnvironment?.set,
		},
	};
}

function globPattern(pattern: string): RegExp {
	let source = "^";
	for (let index = 0; index < pattern.length; index += 1) {
		const character = pattern[index];
		if (character === "*") {
			source += ".*";
		} else if (character === "?") {
			source += ".";
		} else if (character === "[") {
			const close = pattern.indexOf("]", index + 1);
			if (close === -1) {
				source += "\\[";
			} else {
				const contents = pattern.slice(index + 1, close);
				const negated = contents.startsWith("!") || contents.startsWith("^");
				const body = (negated ? contents.slice(1) : contents)
					.replaceAll("\\", "\\\\")
					.replaceAll("]", "\\]");
				source += `[${negated ? "^" : ""}${body}]`;
				index = close;
			}
		} else {
			source += character.replace(/[\\^$.*+?()[\]{}|]/g, "\\$&");
		}
	}
	return new RegExp(`${source}$`, "i");
}

function matchesAny(name: string, patterns: readonly string[]): boolean {
	return patterns.some((pattern) => globPattern(pattern).test(name));
}

export function buildShellEnvironment(
	config: NativeSandboxConfig,
	source: NodeJS.ProcessEnv = process.env,
	packagedMcpCli = PACKAGED_MCP_CLI,
): Record<string, string> {
	const effectiveConfig = mergeGlobalConfig(DEFAULT_CONFIG, config);
	const policy = effectiveConfig.shellEnvironment ?? {};
	const sourceEntries = Object.entries(source).filter(
		(entry): entry is [string, string] => entry[1] !== undefined,
	);
	const inherited =
		policy.inherit === "none"
			? []
			: policy.inherit === "core"
				? sourceEntries.filter(([name]) =>
						UNIX_CORE_ENV_VARS.some((allowed) =>
							allowed.toLowerCase() === name.toLowerCase(),
						),
					)
				: sourceEntries;
	const environment = Object.fromEntries(inherited);

	if (!policy.ignoreDefaultExcludes) {
		for (const name of Object.keys(environment)) {
			if (matchesAny(name, ["*KEY*", "*SECRET*", "*TOKEN*"])) {
				delete environment[name];
			}
		}
	}
	for (const name of Object.keys(environment)) {
		if (matchesAny(name, policy.exclude ?? [])) delete environment[name];
	}
	Object.assign(environment, policy.set ?? {});
	if ((policy.includeOnly?.length ?? 0) > 0) {
		for (const name of Object.keys(environment)) {
			if (!matchesAny(name, policy.includeOnly ?? [])) delete environment[name];
		}
	}
	Object.assign(
		environment,
		developmentCacheEnvironment(effectiveConfig.developmentCache),
	);
	if (packagedMcpCli.length > 0 && !packagedMcpCli.startsWith("@")) {
		environment.PATH = [dirname(packagedMcpCli), environment.PATH]
			.filter(Boolean)
			.join(":");
	}

	return environment;
}
