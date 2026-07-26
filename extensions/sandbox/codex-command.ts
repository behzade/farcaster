export interface CodexSandboxNetworkConfig {
	enabled?: boolean;
	allowedDomains?: string[];
	deniedDomains?: string[];
	allowLocalNetwork?: boolean;
	allowUnixSockets?: string[];
	allowAllUnixSockets?: boolean;
}

export interface CodexSandboxFilesystemConfig {
	allowRead?: string[];
	denyRead?: string[];
	allowWrite?: string[];
	denyWrite?: string[];
}

export interface CodexSandboxGrants {
	read?: readonly string[];
	write?: readonly string[];
	web?: boolean;
	localNetwork?: boolean;
}

export interface CodexSandboxConfig {
	enabled?: boolean;
	codexCommand?: string;
	permissionProfile?: string;
	network?: CodexSandboxNetworkConfig;
	filesystem?: CodexSandboxFilesystemConfig;
}

export const DEFAULT_CONFIG: Required<Pick<CodexSandboxConfig, "enabled" | "codexCommand" | "permissionProfile">> &
	CodexSandboxConfig = {
	enabled: true,
	codexCommand: "codex",
	permissionProfile: "pi-sandbox",
	network: {
		enabled: true,
		allowedDomains: [
			"npmjs.org",
			"*.npmjs.org",
			"registry.npmjs.org",
			"registry.yarnpkg.com",
			"pypi.org",
			"*.pypi.org",
			"github.com",
			"*.github.com",
			"api.github.com",
			"raw.githubusercontent.com",
			"opencode.ai",
			"*.opencode.ai",
			"models.dev",
		],
		deniedDomains: [],
		allowLocalNetwork: false,
		allowUnixSockets: [],
		allowAllUnixSockets: false,
	},
	filesystem: {
		allowRead: [":root"],
		denyRead: [
			"~/.ssh",
			"~/.aws",
			"~/.gnupg",
			"/**/.env",
			"/**/.env.*",
			"/**/*.key",
		],
		allowWrite: [".", ":tmpdir", ":slash_tmp"],
		denyWrite: [
			".env",
			".env.*",
			"*.pem",
			"*.key",
			"~/.pi/agent",
			"~/.codex",
		],
	},
};

const PROFILE_NAME = /^[A-Za-z0-9_-]+$/;

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

function assertKnownKeys(value: Record<string, unknown>, allowed: readonly string[], field: string): void {
	const unknown = Object.keys(value).filter((key) => !allowed.includes(key));
	if (unknown.length > 0) throw new Error(`${field} contains unknown fields: ${unknown.join(", ")}`);
}

export function normalizeConfig(value: unknown): CodexSandboxConfig {
	if (!value || typeof value !== "object" || Array.isArray(value)) {
		throw new Error("sandbox config must be a JSON object");
	}
	const input = value as Record<string, unknown>;
	assertKnownKeys(
		input,
		["enabled", "codexCommand", "permissionProfile", "network", "filesystem", "allowPty"],
		"sandbox config",
	);
	const enabled = input.enabled;
	const codexCommand = input.codexCommand;
	const permissionProfile = input.permissionProfile;
	if (enabled !== undefined && typeof enabled !== "boolean") {
		throw new Error("enabled must be a boolean");
	}
	if (codexCommand !== undefined && (typeof codexCommand !== "string" || codexCommand.length === 0)) {
		throw new Error("codexCommand must be a non-empty string");
	}
	if (
		permissionProfile !== undefined &&
		(typeof permissionProfile !== "string" || !PROFILE_NAME.test(permissionProfile))
	) {
		throw new Error("permissionProfile must contain only letters, digits, underscores, and hyphens");
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
	if (
		networkInput?.allowLocalNetwork !== undefined &&
		typeof networkInput.allowLocalNetwork !== "boolean"
	) {
		throw new Error("network.allowLocalNetwork must be a boolean");
	}
	if (networkInput) {
		assertKnownKeys(
			networkInput,
			[
				"enabled",
				"allowedDomains",
				"deniedDomains",
				"allowLocalNetwork",
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

	return {
		enabled: enabled as boolean | undefined,
		codexCommand: codexCommand as string | undefined,
		permissionProfile: permissionProfile as string | undefined,
		network: networkInput
			? {
					enabled: networkInput.enabled as boolean | undefined,
					allowedDomains: domainPatterns(networkInput.allowedDomains, "network.allowedDomains"),
					deniedDomains: domainPatterns(networkInput.deniedDomains, "network.deniedDomains"),
					allowLocalNetwork: networkInput.allowLocalNetwork as boolean | undefined,
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
	};
}

export function mergeGlobalConfig(
	defaults: CodexSandboxConfig,
	override: CodexSandboxConfig,
): CodexSandboxConfig {
	const defined = <T extends object>(value: T | undefined): Partial<T> =>
		Object.fromEntries(
			Object.entries(value ?? {}).filter(([, entry]) => entry !== undefined),
		) as Partial<T>;
	return {
		...defaults,
		...defined(override),
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
	};
}

export function applyProjectRestrictions(
	base: CodexSandboxConfig,
	project: CodexSandboxConfig,
): CodexSandboxConfig {
	return {
		...base,
		// A project file may tighten the active profile, but it may not turn off
		// the host sandbox or add rights.
		enabled: base.enabled,
		network: {
			...base.network,
			enabled:
				base.network?.enabled === false || project.network?.enabled === false
					? false
					: base.network?.enabled,
			allowedDomains: base.network?.allowedDomains,
			allowLocalNetwork:
				base.network?.allowLocalNetwork === false ||
				project.network?.allowLocalNetwork === false
					? false
					: base.network?.allowLocalNetwork,
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
	};
}

function tomlString(value: string): string {
	return JSON.stringify(value);
}

type FilesystemAccess = "write" | "read" | "deny";

const ACCESS_RANK: Record<FilesystemAccess, number> = {
	write: 0,
	read: 1,
	deny: 2,
};

function setStrictestAccess(
	entries: Map<string, FilesystemAccess>,
	path: string,
	access: FilesystemAccess,
): void {
	const current = entries.get(path);
	if (!current || ACCESS_RANK[access] > ACCESS_RANK[current]) entries.set(path, access);
}

function containsGlob(path: string): boolean {
	return path.includes("*") || path.includes("?") || path.includes("[");
}

function configOverride(path: string, value: string | boolean): string {
	const encoded = typeof value === "string" ? tomlString(value) : String(value);
	return `${path}=${encoded}`;
}

function rawConfigOverride(path: string, tomlValue: string): string {
	return `${path}=${tomlValue}`;
}

interface RawToml {
	raw: string;
}

type TomlValue = string | boolean | RawToml;

function rawToml(raw: string): RawToml {
	return { raw };
}

function inlineTable(entries: readonly [string, TomlValue][]): string {
	return `{ ${entries
		.map(([key, value]) => {
			const encoded =
				typeof value === "string"
					? tomlString(value)
					: typeof value === "boolean"
						? String(value)
						: value.raw;
			return `${tomlString(key)} = ${encoded}`;
		})
		.join(", ")} }`;
}

export function buildCodexSandboxArgs(
	cwd: string,
	config: CodexSandboxConfig,
	command: string,
	grants: CodexSandboxGrants = {},
): string[] {
	const effectiveConfig = mergeGlobalConfig(DEFAULT_CONFIG, config);
	const profileBase = effectiveConfig.permissionProfile ?? DEFAULT_CONFIG.permissionProfile;
	if (!PROFILE_NAME.test(profileBase)) {
		throw new Error("invalid Codex permission profile name");
	}
	// A per-process name prevents an unrelated profile in the user's Codex
	// config from merging extra permissions into this generated profile.
	const profile = `${profileBase}-${process.pid}`;

	const filesystemEntries = new Map<string, FilesystemAccess>();
	for (const path of [...(effectiveConfig.filesystem?.allowRead ?? []), ...(grants.read ?? [])]) {
		filesystemEntries.set(path, "read");
	}
	for (const path of [...(effectiveConfig.filesystem?.allowWrite ?? []), ...(grants.write ?? [])]) {
		filesystemEntries.set(path, "write");
	}
	for (const path of effectiveConfig.filesystem?.denyWrite ?? []) {
		setStrictestAccess(filesystemEntries, path, "read");
	}
	for (const path of effectiveConfig.filesystem?.denyRead ?? []) {
		setStrictestAccess(filesystemEntries, path, "deny");
	}
	const directFilesystemEntries: [string, TomlValue][] = [];
	const workspaceFilesystemEntries: [string, TomlValue][] = [];
	for (const [path, requestedAccess] of [...filesystemEntries].sort(([left], [right]) =>
		left.localeCompare(right),
	)) {
		// Codex accepts glob paths only for deny rules. Tighten a read-only glob
		// to deny rather than reject the whole sandbox or permit writes.
		const access = requestedAccess === "read" && containsGlob(path) ? "deny" : requestedAccess;
		if (path.startsWith("/") || path.startsWith("~") || path.startsWith(":")) {
			directFilesystemEntries.push([path, access]);
		} else {
			workspaceFilesystemEntries.push([path, access]);
		}
	}
	if (workspaceFilesystemEntries.length > 0) {
		directFilesystemEntries.push([
			":workspace_roots",
			rawToml(inlineTable(workspaceFilesystemEntries)),
		]);
	}
	const networkEnabled = effectiveConfig.network?.enabled ?? false;
	const localNetworkEnabled =
		networkEnabled &&
		((effectiveConfig.network?.allowLocalNetwork ?? false) ||
			(grants.localNetwork ?? false));
	const domainEntries = new Map<string, "allow" | "deny">();
	for (const domain of unique(effectiveConfig.network?.allowedDomains ?? [])) {
		domainEntries.set(domain, "allow");
	}
	if (grants.web && networkEnabled) domainEntries.set("*", "allow");
	if (localNetworkEnabled) {
		domainEntries.set("localhost", "allow");
		domainEntries.set("127.0.0.1", "allow");
		domainEntries.set("::1", "allow");
	}
	for (const domain of unique(effectiveConfig.network?.deniedDomains ?? [])) {
		domainEntries.set(domain, "deny");
	}
	const networkEntries: [string, TomlValue][] = [["enabled", networkEnabled]];
	if (networkEnabled) {
		networkEntries.push(
			["mode", "full"],
			["allow_local_binding", localNetworkEnabled],
			[
				"domains",
				rawToml(inlineTable(
					[...domainEntries]
						.sort(([left], [right]) => left.localeCompare(right))
						.map(([domain, access]): [string, TomlValue] => [domain, access]),
				)),
			],
		);
		const sockets = unique(effectiveConfig.network?.allowUnixSockets ?? []).sort();
		if (sockets.length > 0) {
			networkEntries.push([
				"unix_sockets",
				rawToml(
					inlineTable(
						sockets.map((socket): [string, TomlValue] => [socket, "allow"]),
					),
				),
			]);
		}
		if (effectiveConfig.network?.allowAllUnixSockets) {
			networkEntries.push(["dangerously_allow_all_unix_sockets", true]);
		}
	}

	const profileValue = inlineTable([
		["extends", ":workspace"],
		[
			"filesystem",
			rawToml(
				inlineTable(directFilesystemEntries),
			),
		],
		["network", rawToml(inlineTable(networkEntries))],
	]);
	const overrides = [
		rawConfigOverride(`permissions.${profile}`, profileValue),
		...(networkEnabled ? [configOverride("features.network_proxy", true)] : []),
	];

	return [
		...overrides.flatMap((override) => ["-c", override]),
		"sandbox",
		"--permission-profile",
		profile,
		"--cd",
		cwd,
		"--include-managed-config",
		"--",
		"bash",
		"-c",
		command,
	];
}
