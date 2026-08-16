import { isIP } from "node:net";
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
const DEFAULT_SECRET_ENV_PATTERNS = [
	"*KEY*",
	"*SECRET*",
	"*TOKEN*",
	"*PASSWORD*",
	"*PASSWD*",
	"*CREDENTIAL*",
] as const;
const SHELL_CORE_ENV_PATTERNS = [
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
	"AR",
	"AS",
	"BASH",
	"BINDGEN_EXTRA_CLANG_ARGS",
	"CARGO_*",
	"CC",
	"CONFIG_SHELL",
	"CPATH",
	"CPLUS_INCLUDE_PATH",
	"C_INCLUDE_PATH",
	"CXX",
	"DEVELOPER_DIR",
	"HOST_PATH",
	"IN_NIX_SHELL",
	"LD",
	"LD_DYLD_PATH",
	"LD_LIBRARY_PATH",
	"LIBCLANG_PATH",
	"LIBRARY_PATH",
	"MACOSX_DEPLOYMENT_TARGET",
	"NIX_APPLE_SDK_VERSION",
	"NIX_BINTOOLS",
	"NIX_BINTOOLS_WRAPPER_*",
	"NIX_BUILD_CORES",
	"NIX_CC",
	"NIX_CC_WRAPPER_*",
	"NIX_CFLAGS_COMPILE",
	"NIX_DONT_SET_RPATH",
	"NIX_DONT_SET_RPATH_FOR_BUILD",
	"NIX_ENFORCE_NO_NATIVE",
	"NIX_HARDENING_ENABLE",
	"NIX_IGNORE_LD_THROUGH_GCC",
	"NIX_LDFLAGS",
	"NIX_LD_LIBRARY_PATH",
	"NIX_NO_SELF_RPATH",
	"NIX_PKG_CONFIG_WRAPPER_*",
	"NIX_STORE",
	"NM",
	"OBJCOPY",
	"OBJDUMP",
	"PKG_CONFIG*",
	"RANLIB",
	"RUST*",
	"SDKROOT",
	"SIZE",
	"SOURCE_DATE_EPOCH",
	"STRINGS",
	"STRIP",
	"ZERO_AR_DATE",
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
	if (!entries) return undefined;
	try {
		return unique(entries.map((entry) => {
			if (entry === "*") return entry;
			const prefix = entry.startsWith("**.") ? "**." : entry.startsWith("*.") ? "*." : "";
			const host = prefix ? entry.slice(prefix.length) : entry;
			if (host.includes("*")) throw new Error("wildcards are allowed only as *. or **. prefixes");
			const normalizedHost = normalizeNetworkHost(host);
			if (prefix && isIP(normalizedHost)) throw new Error("wildcard prefixes require a domain name");
			return `${prefix}${normalizedHost}`;
		}));
	} catch (error) {
		throw new Error(
			`${field} accepts only exact hosts, *, *.domain, or **.domain: ${
				error instanceof Error ? error.message : error
			}`,
		);
	}
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
						matchesAny(name, SHELL_CORE_ENV_PATTERNS),
					)
				: sourceEntries;
	const environment = Object.fromEntries(inherited);

	if (!policy.ignoreDefaultExcludes) {
		for (const name of Object.keys(environment)) {
			if (matchesAny(name, DEFAULT_SECRET_ENV_PATTERNS)) delete environment[name];
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
