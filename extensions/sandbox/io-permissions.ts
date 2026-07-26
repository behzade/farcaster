import {
	existsSync,
	mkdirSync,
	readFileSync,
	realpathSync,
	renameSync,
	statSync,
	writeFileSync,
} from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";

export type IoPermission =
	| {
			kind: "read" | "write";
			path: string;
			directory: boolean;
	  }
	| {
			kind: "web";
	  }
	| {
			kind: "local_network";
	  }
	| {
			kind: "mcp";
			server: string;
	  };

export interface RuntimeIoGrants {
	read: string[];
	write: string[];
	web: boolean;
	localNetwork: boolean;
}

interface PermissionFile {
	version: 1;
	workspaces: Record<string, IoPermission[]>;
}

const EMPTY_FILE: PermissionFile = { version: 1, workspaces: {} };
const protectedHomeRoots = [".ssh", ".aws", ".gnupg"];
const protectedWriteRoots = [".pi/agent", ".codex"];
const secretNames = [/^\.env(?:\..*)?$/, /\.(?:pem|key)$/];

export function canonicalize(path: string): string {
	if (existsSync(path)) return realpathSync.native(path);
	const parent = dirname(path);
	if (parent === path) return resolve(path);
	return resolve(canonicalize(parent), path.slice(parent.length + (parent.endsWith(sep) ? 0 : 1)));
}

export function resolvePermissionPath(path: string, cwd: string): string {
	const expanded =
		path === "~"
			? homedir()
			: path.startsWith("~/")
				? resolve(homedir(), path.slice(2))
				: isAbsolute(path)
					? path
					: resolve(cwd, path);
	return canonicalize(expanded);
}

export function isInside(root: string, path: string): boolean {
	const rel = relative(root, path);
	return rel === "" || (rel !== ".." && !rel.startsWith(`..${sep}`));
}

export function isProtectedPath(path: string): boolean {
	const actual = canonicalize(path);
	const home = canonicalize(homedir());
	if (
		protectedHomeRoots.some((name) =>
			isInside(canonicalize(resolve(home, name)), actual),
		)
	) {
		return true;
	}
	if (actual.split(sep).some((part) => secretNames.some((pattern) => pattern.test(part)))) {
		return true;
	}
	return false;
}

export function isProtectedWritePath(path: string): boolean {
	if (isProtectedPath(path)) return true;
	const actual = canonicalize(path);
	if (actual.split(sep).includes(".git")) return true;
	const home = canonicalize(homedir());
	return protectedWriteRoots.some((name) =>
		isInside(canonicalize(resolve(home, name)), actual),
	);
}

export function normalizePermission(
	input:
		| { kind: "read" | "write"; path: string; targetType?: "file" | "folder" }
		| { kind: "web" }
		| { kind: "local_port"; port: number },
	cwd: string,
): IoPermission {
	if (input.kind === "web") return { kind: "web" };
	if (input.kind === "local_port") return { kind: "local_network" };
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
	if (permission.kind === "web") return false;
	if (permission.kind === "local_network") return false;
	if (permission.kind === "mcp") return false;
	const target = canonicalize(path);
	return permission.directory ? isInside(permission.path, target) : permission.path === target;
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
	let web = false;
	let localNetwork = false;
	for (const permission of permissions) {
		if (permission.kind === "read") read.add(permission.path);
		if (permission.kind === "write") write.add(permission.path);
		if (permission.kind === "web") web = true;
		if (permission.kind === "local_network") localNetwork = true;
	}
	return { read: [...read].sort(), write: [...write].sort(), web, localNetwork };
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
	if (permission.kind === "web") return "public web access";
	if (permission.kind === "local_network") {
		return "localhost, private-network, and link-local access on all ports";
	}
	if (permission.kind === "mcp") return `MCP service ${permission.server}`;
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
		if (record.version !== 1 || !record.workspaces || typeof record.workspaces !== "object") {
			return structuredClone(EMPTY_FILE);
		}
		const workspaces: Record<string, IoPermission[]> = {};
		for (const [workspace, entries] of Object.entries(record.workspaces as Record<string, unknown>)) {
			if (!Array.isArray(entries)) continue;
			workspaces[workspace] = entries.filter(isIoPermission);
		}
		return { version: 1, workspaces };
	} catch {
		return structuredClone(EMPTY_FILE);
	}
}

function isIoPermission(value: unknown): value is IoPermission {
	if (!value || typeof value !== "object" || Array.isArray(value)) return false;
	const record = value as Record<string, unknown>;
	if (record.kind === "web") return Object.keys(record).every((key) => key === "kind");
	if (record.kind === "local_network") {
		return Object.keys(record).every((key) => key === "kind");
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
