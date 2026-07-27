import {
	existsSync,
	lstatSync,
	mkdirSync,
	readFileSync,
	realpathSync,
	renameSync,
	statSync,
	writeFileSync,
} from "node:fs";
import { isIP } from "node:net";
import { homedir, tmpdir } from "node:os";
import { basename, dirname, isAbsolute, relative, resolve, sep } from "node:path";
import { domainToASCII } from "node:url";

export type IoPermission =
	| {
			kind: "read" | "write";
			path: string;
			directory: boolean;
	  }
	| {
			kind: "network_host";
			host: string;
	  }
	| {
			kind: "mcp";
			server: string;
	  };

export interface RuntimeIoGrants {
	read: string[];
	write: string[];
	networkHosts: string[];
}

interface PermissionFile {
	version: 2;
	workspaces: Record<string, IoPermission[]>;
}

const EMPTY_FILE: PermissionFile = { version: 2, workspaces: {} };
const protectedHomeRoots = [".ssh", ".aws", ".gnupg"];
const protectedWriteRoots = [".pi", ".codex"];
const protectedAuthFiles = [".pi/agent/auth.json", ".codex/auth.json"];
const secretNames = [/^\.env(?:\..*)?$/, /\.(?:pem|key)$/];

export function canonicalize(path: string): string {
	if (existsSync(path)) return realpathSync.native(path);
	const parent = dirname(path);
	if (parent === path) return resolve(path);
	return resolve(canonicalize(parent), path.slice(parent.length + (parent.endsWith(sep) ? 0 : 1)));
}

function expandPermissionPath(path: string, cwd: string): string {
	return path === "~"
		? homedir()
		: path.startsWith("~/")
			? resolve(homedir(), path.slice(2))
			: isAbsolute(path)
				? resolve(path)
				: resolve(cwd, path);
}

export function resolveLexicalPermissionPath(path: string, cwd: string): string {
	return expandPermissionPath(path, cwd);
}

export function resolvePermissionPath(path: string, cwd: string): string {
	return canonicalize(expandPermissionPath(path, cwd));
}

export function isInside(root: string, path: string): boolean {
	const rel = relative(root, path);
	return rel === "" || (rel !== ".." && !rel.startsWith(`..${sep}`));
}

export function isProtectedPath(path: string): boolean {
	const home = canonicalize(homedir());
	const lexical = resolve(path);
	const protectedRoots = protectedHomeRoots.map((name) => resolve(home, name));
	const protectedFiles = protectedAuthFiles.map((name) => resolve(home, name));
	// Check fixed host paths before realpath. The OS sandbox may make those
	// roots unreadable even to a test process, and a protected path must still
	// be recognized without probing it.
	if (protectedRoots.some((root) => isInside(root, lexical))) return true;
	if (protectedFiles.includes(lexical)) return true;

	const actual = canonicalize(path);
	if (protectedRoots.some((root) => isInside(root, actual))) return true;
	if (protectedFiles.includes(actual)) return true;
	return actual
		.split(sep)
		.some((part) => secretNames.some((pattern) => pattern.test(part)));
}

export function isProtectedWritePath(path: string): boolean {
	if (isProtectedPath(path)) return true;
	const home = canonicalize(homedir());
	const lexical = resolve(path);
	const protectedRoots = protectedWriteRoots.map((name) => resolve(home, name));
	if (protectedRoots.some((root) => isInside(root, lexical))) return true;
	const actual = canonicalize(path);
	return protectedRoots.some((root) => isInside(root, actual));
}

function namedControlRoot(path: string, name: ".git"): string | undefined {
	const parts = resolve(path).split(sep);
	const index = parts.lastIndexOf(name);
	if (index < 0) return undefined;
	const root = parts.slice(0, index + 1).join(sep);
	return root || sep;
}

export function gitControlRoot(path: string, cwd?: string): string | undefined {
	// Keep the lexical check first. Canonicalizing a `.git` symlink removes the
	// control name and could otherwise make it look like a normal workspace path.
	const lexical = namedControlRoot(path, ".git");
	if (lexical) {
		const canonicalRoot = canonicalize(lexical);
		return basename(canonicalRoot) === ".git" ? canonicalRoot : lexical;
	}
	const actual = canonicalize(path);
	const namedActual = namedControlRoot(actual, ".git");
	if (namedActual) return namedActual;
	if (cwd) {
		const workspaceRoot = resolve(cwd, ".git");
		if (isInside(canonicalize(workspaceRoot), actual)) return workspaceRoot;
	}
	return undefined;
}

export function projectControlRoot(path: string, cwd: string): string | undefined {
	const root = resolve(cwd, ".pi");
	const canonicalRoot = canonicalize(root);
	const returnedRoot = basename(canonicalRoot) === ".pi" ? canonicalRoot : root;
	const lexical = resolve(path);
	if (isInside(root, lexical)) return returnedRoot;
	return isInside(canonicalRoot, canonicalize(path)) ? returnedRoot : undefined;
}

export function isControlRootSymlink(path: string): boolean {
	if (basename(path) !== ".git" && basename(path) !== ".pi") return false;
	try {
		return lstatSync(path).isSymbolicLink();
	} catch {
		return false;
	}
}

export function normalizeNetworkHost(input: string): string {
	let value = input.trim();
	if (value.startsWith("[") && value.endsWith("]")) {
		value = value.slice(1, -1);
		if (!isIP(value)) throw new Error("Invalid IP address");
	}
	if (isIP(value)) return value.toLowerCase();
	if (
		value.length === 0 ||
		value.includes("*") ||
		value.includes("/") ||
		value.includes(":") ||
		value.includes("?") ||
		value.includes("#") ||
		value.includes("@")
	) {
		throw new Error("Network access needs one exact hostname or IP without a scheme, port, path, or wildcard");
	}
	value = value.replace(/\.$/, "").toLowerCase();
	if (/^[0-9.]+$/.test(value)) {
		throw new Error("Invalid IP address");
	}
	const ascii = domainToASCII(value);
	const labels = ascii.split(".");
	if (
		ascii.length === 0 ||
		ascii.length > 253 ||
		labels.some(
			(label) =>
				label.length === 0 ||
				label.length > 63 ||
				!/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/.test(label),
		)
	) {
		throw new Error("Invalid hostname");
	}
	return ascii;
}

export function normalizePermission(
	input:
		| { kind: "read" | "write"; path: string; targetType?: "file" | "folder" }
		| { kind: "network_host"; host: string },
	cwd: string,
): IoPermission {
	if (input.kind === "network_host") {
		return { kind: "network_host", host: normalizeNetworkHost(input.host) };
	}
	const lexicalPath = expandPermissionPath(input.path, cwd);
	if (
		isProtectedPath(lexicalPath) ||
		(input.kind === "write" && isProtectedWritePath(lexicalPath))
	) {
		throw new Error(`Protected secret or control path cannot be granted: ${lexicalPath}`);
	}
	const path = resolvePermissionPath(input.path, cwd);
	if (
		isProtectedPath(path) ||
		(input.kind === "write" && isProtectedWritePath(path))
	) {
		throw new Error(`Protected secret or control path cannot be granted: ${path}`);
	}
	const directory =
		input.targetType === "folder" ||
		(input.targetType === undefined && existsSync(path) && statSync(path).isDirectory());
	return { kind: input.kind, path, directory };
}

export function permissionCoversPath(permission: IoPermission, path: string): boolean {
	if (permission.kind === "network_host") return false;
	if (permission.kind === "mcp") return false;
	const root = canonicalize(permission.path);
	const target = canonicalize(path);
	return permission.directory ? isInside(root, target) : root === target;
}

export function mcpPermissionFromInput(input: unknown): IoPermission | undefined {
	if (!input || typeof input !== "object" || Array.isArray(input)) return undefined;
	const record = input as Record<string, unknown>;
	for (const key of [
		"name",
		"server",
		"serverName",
		"serverId",
		"server_name",
		"server_id",
		"id",
	]) {
		const value = record[key];
		if (typeof value === "string" && value.trim().length > 0) {
			return { kind: "mcp", server: value.trim() };
		}
	}
	return undefined;
}

export function grantsToRuntime(permissions: readonly IoPermission[]): RuntimeIoGrants {
	const read = new Set<string>();
	const write = new Set<string>();
	const networkHosts = new Set<string>();
	for (const permission of permissions) {
		if (permission.kind === "read") read.add(canonicalize(permission.path));
		if (permission.kind === "write" && !isControlRootSymlink(permission.path)) {
			write.add(canonicalize(permission.path));
		}
		if (permission.kind === "network_host") networkHosts.add(permission.host);
	}
	return {
		read: [...read].sort(),
		write: [...write].sort(),
		networkHosts: [...networkHosts].sort(),
	};
}

export function isDefaultWritePath(path: string, cwd: string): boolean {
	const actual = canonicalize(path);
	return [
		canonicalize(cwd),
		canonicalize("/tmp"),
		canonicalize(tmpdir()),
	].some((root) => isInside(root, actual));
}

export function permissionLabel(permission: IoPermission): string {
	if (permission.kind === "network_host") return `network host ${permission.host}`;
	if (permission.kind === "mcp") return `MCP service ${permission.server}`;
	if (permission.kind === "write" && permission.directory && basename(permission.path) === ".pi") {
		return `write Pi project control folder ${permission.path} (code there can run on reload)`;
	}
	return `${permission.kind} ${permission.directory ? "folder" : "file"} ${permission.path}`;
}

export function loadWorkspacePermissions(filePath: string, cwd: string): IoPermission[] {
	const file = readPermissionFile(filePath);
	const workspace = canonicalize(cwd);
	return [...(file.workspaces[workspace] ?? [])];
}

export function saveWorkspacePermission(
	filePath: string,
	cwd: string,
	permission: IoPermission,
): void {
	const file = readPermissionFile(filePath);
	const workspace = canonicalize(cwd);
	const existing = file.workspaces[workspace] ?? [];
	const key = JSON.stringify(permission);
	if (!existing.some((entry) => JSON.stringify(entry) === key)) {
		file.workspaces[workspace] = [...existing, permission];
	}
	mkdirSync(dirname(filePath), { recursive: true, mode: 0o700 });
	const temporary = `${filePath}.${process.pid}.tmp`;
	writeFileSync(temporary, `${JSON.stringify(file, null, 2)}\n`, { mode: 0o600 });
	renameSync(temporary, filePath);
}

function readPermissionFile(filePath: string): PermissionFile {
	if (!existsSync(filePath)) return structuredClone(EMPTY_FILE);
	try {
		const value: unknown = JSON.parse(readFileSync(filePath, "utf8"));
		if (!value || typeof value !== "object" || Array.isArray(value)) return structuredClone(EMPTY_FILE);
		const record = value as Record<string, unknown>;
		if (
			(record.version !== 1 && record.version !== 2) ||
			!record.workspaces ||
			typeof record.workspaces !== "object"
		) {
			return structuredClone(EMPTY_FILE);
		}
		const workspaces: Record<string, IoPermission[]> = {};
		for (const [workspace, entries] of Object.entries(record.workspaces as Record<string, unknown>)) {
			if (!Array.isArray(entries)) continue;
			workspaces[workspace] = entries.filter(isIoPermission);
		}
		return { version: 2, workspaces };
	} catch {
		return structuredClone(EMPTY_FILE);
	}
}

function isIoPermission(value: unknown): value is IoPermission {
	if (!value || typeof value !== "object" || Array.isArray(value)) return false;
	const record = value as Record<string, unknown>;
	if (record.kind === "network_host") {
		if (
			!Object.keys(record).every((key) => key === "kind" || key === "host") ||
			typeof record.host !== "string"
		) {
			return false;
		}
		try {
			return normalizeNetworkHost(record.host) === record.host;
		} catch {
			return false;
		}
	}
	if (record.kind === "mcp") {
		return (
			Object.keys(record).every((key) => key === "kind" || key === "server") &&
			typeof record.server === "string" &&
			record.server.length > 0
		);
	}
	return (
		(record.kind === "read" || record.kind === "write") &&
		typeof record.path === "string" &&
		isAbsolute(record.path) &&
		typeof record.directory === "boolean"
	);
}
