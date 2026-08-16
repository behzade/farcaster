import { accessSync, constants, existsSync, statSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { delimiter, isAbsolute, join, resolve } from "node:path";
import type {
	BrokerExecRequest,
	BrokerFilesystemDeny,
	BrokerFilesystemRight,
} from "./broker-client.ts";
import { developmentCacheWriteRightsForWorkspace } from "./development-caches.ts";
import {
	DEFAULT_CONFIG,
	buildShellEnvironment,
	mergeGlobalConfig,
	type NativeSandboxConfig,
} from "./sandbox-config.ts";
import {
	canonicalize,
	type IoPermission,
	resolvePermissionPath,
} from "./io-permissions.ts";

const OUTPUT_LIMIT_BYTES = 10 * 1024 * 1024;

export type NativeFilePermission = IoPermission;

export function buildBrokerExecRequest(
	id: string,
	command: string,
	cwd: string,
	timeoutSeconds: number | undefined,
	config: NativeSandboxConfig,
	permissions: readonly NativeFilePermission[],
	networkHosts: readonly string[],
	proxy?: { port: number; socketPath: string },
	allowLocalBinding = false,
): BrokerExecRequest {
	const effective = mergeGlobalConfig(DEFAULT_CONFIG, config);
	if ((networkHosts.length > 0) !== (proxy !== undefined)) {
		throw new Error("Native network hosts and proxy state must be provided together");
	}
	if (effective.network?.allowAllUnixSockets) {
		throw new Error("The native sandbox does not support allowing all Unix sockets");
	}
	if (
		timeoutSeconds !== undefined &&
		(!Number.isFinite(timeoutSeconds) || timeoutSeconds > 86_400)
	) {
		throw new Error("Native sandbox timeout must be finite and no more than 24 hours");
	}
	const actualCwd = canonicalize(cwd);
	return {
		type: "exec",
		id,
		command: { program: hostBash(), args: ["-c", command] },
		cwd: actualCwd,
		env: {
			...buildShellEnvironment(effective),
			IN_SANDBOX: "1",
			PI_SANDBOX: "seatbelt-broker",
		},
		timeout_ms:
			timeoutSeconds === undefined || timeoutSeconds <= 0
				? null
				: Math.max(1, Math.round(timeoutSeconds * 1000)),
		interactive: false,
		policy: {
			base_rights: baseRights(effective, actualCwd),
			grants: permissions.map(permissionRight),
			denies: denyRules(effective, actualCwd),
			network: proxy
				? {
						mode: "proxy",
						tcp_port: proxy.port,
						unix_socket: proxy.socketPath,
						allow_local_binding: allowLocalBinding,
					}
				: allowLocalBinding
					? { mode: "loopback" }
					: { mode: "blocked" },
			unix_socket_roots: unixSocketRoots(effective),
			output_limit_bytes: OUTPUT_LIMIT_BYTES,
		},
	};
}

function hostBash(): string {
	for (const directory of (process.env.PATH ?? "").split(delimiter)) {
		if (!directory) continue;
		const candidate = join(directory, "bash");
		try {
			accessSync(candidate, constants.X_OK);
			return canonicalize(candidate);
		} catch {
			// Continue to the fixed fallback.
		}
	}
	return canonicalize("/bin/bash");
}

function unixSocketRoots(config: NativeSandboxConfig): string[] {
	const roots = new Set<string>();
	for (const socket of config.network?.allowUnixSockets ?? []) {
		if (!isAbsolute(socket)) {
			throw new Error(`Native sandbox Unix socket paths must be absolute: ${socket}`);
		}
		const path = canonicalize(socket);
		roots.add(path);
	}
	return [...roots].sort();
}

function baseRights(
	config: NativeSandboxConfig,
	cwd: string,
): BrokerFilesystemRight[] {
	const rights = new Map<string, BrokerFilesystemRight>();
	for (const cache of developmentCacheWriteRightsForWorkspace(
		cwd,
		config.developmentCache,
	)) {
		const right: BrokerFilesystemRight = {
			access: "write",
			path: cache.path,
			scope: cache.directory ? "tree" : "file",
			missing_path: existsSync(cache.path)
				? "reject"
				: cache.directory
					? "create_tree"
					: "create_file",
		};
		rights.set(`write:${right.path}:${right.scope}`, right);
	}
	for (const entry of config.filesystem?.allowRead ?? []) {
		const right = configRight("read", entry, cwd);
		if (right) rights.set(`read:${right.path}:${right.scope}`, right);
	}
	for (const entry of config.filesystem?.allowWrite ?? []) {
		const right = configRight("write", entry, cwd);
		if (right) rights.set(`write:${right.path}:${right.scope}`, right);
	}
	return [...rights.values()];
}

function configRight(
	access: "read" | "write",
	entry: string,
	cwd: string,
): BrokerFilesystemRight | undefined {
	let path: string;
	if (entry === ":root") path = "/";
	else if (entry === "." || entry === ":workspace_roots") path = cwd;
	else if (entry === ":tmpdir") path = canonicalize(tmpdir());
	else if (entry === ":slash_tmp") path = canonicalize("/tmp");
	else if (entry.startsWith(":")) return undefined;
	else if (containsGlob(entry)) {
		throw new Error(`Native sandbox read/write roots cannot contain globs: ${entry}`);
	} else path = resolvePermissionPath(entry, cwd);

	if (access === "read" && !existsSync(path)) return undefined;
	const directory = path === "/" || !existsSync(path) || statSync(path).isDirectory();
	return {
		access,
		path,
		scope: directory ? "tree" : "file",
		missing_path: existsSync(path)
			? "reject"
			: directory
				? "create_tree"
				: "create_file",
	};
}

function permissionRight(permission: NativeFilePermission): BrokerFilesystemRight {
	return {
		access: permission.kind,
		path: permission.path,
		scope: permission.directory ? "tree" : "file",
		missing_path: existsSync(permission.path)
			? "reject"
			: permission.directory
				? "create_tree"
				: "create_file",
	};
}

function denyRules(
	config: NativeSandboxConfig,
	cwd: string,
): BrokerFilesystemDeny[] {
	const rules = new Map<string, BrokerFilesystemDeny>();
	for (const [access, entries] of [
		["read" as const, config.filesystem?.denyRead ?? []],
		["write" as const, config.filesystem?.denyWrite ?? []],
	] as const) {
		for (const entry of entries) {
			const normalized = normalizeDeny(entry, cwd);
			const key = `${normalized.pattern}:${normalized.scope}`;
			const current = rules.get(key);
			rules.set(key, {
				...normalized,
				access:
					current && current.access !== access
						? "read_write"
						: current?.access ?? access,
			});
		}
	}
	return [...rules.values()];
}

function normalizeDeny(
	entry: string,
	cwd: string,
): Omit<BrokerFilesystemDeny, "access"> {
	if (containsGlob(entry)) {
		assertGlobHasNoDotSegments(entry);
		let pattern: string;
		if (entry.startsWith("~/")) pattern = `${homedir()}/${entry.slice(2)}`;
		else if (isAbsolute(entry)) pattern = entry;
		else pattern = `${cwd}/${entry.includes("/") ? entry : `**/${entry}`}`;
		return { pattern, scope: "glob" };
	}
	const pattern = lexicalPath(entry, cwd);
	let scope: "file" | "tree" = "tree";
	try {
		if (existsSync(pattern) && !statSync(pattern).isDirectory()) scope = "file";
	} catch {
		// An unreadable deny root stays a tree deny without probing it further.
	}
	return { pattern, scope };
}

function lexicalPath(path: string, cwd: string): string {
	if (path === "~") return homedir();
	if (path.startsWith("~/")) return resolve(homedir(), path.slice(2));
	return isAbsolute(path) ? resolve(path) : resolve(cwd, path);
}

function assertGlobHasNoDotSegments(value: string): void {
	if (value.split("/").some((part) => part === "." || part === "..")) {
		throw new Error(`Native sandbox deny globs cannot contain . or .. segments: ${value}`);
	}
}

function containsGlob(value: string): boolean {
	return value.includes("*") || value.includes("?") || value.includes("[");
}
