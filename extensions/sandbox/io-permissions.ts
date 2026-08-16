import {
	existsSync,
	lstatSync,
	realpathSync,
	statSync,
} from "node:fs";
import { isIP } from "node:net";
import { homedir, tmpdir } from "node:os";
import { basename, dirname, isAbsolute, relative, resolve, sep } from "node:path";
import { domainToASCII } from "node:url";
import {
	developmentCacheRightForPath,
	type DevelopmentCacheConfig,
} from "./development-caches.ts";

export interface IoPermission {
	kind: "read" | "write";
	path: string;
	directory: boolean;
}
const protectedHomeRoots = [".ssh", ".aws", ".gnupg"];
const protectedSystemRoots = ["/dev"];
const protectedWriteRoots = [".pi", ".codex"];
const protectedAuthFiles = [
	".pi/agent/auth.json",
	".pi/agent/extensions/sandbox.json",
	".codex/auth.json",
];
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
	const protectedRoots = [
		...protectedHomeRoots.map((name) => resolve(home, name)),
		...protectedSystemRoots.map((path) => resolve(path)),
	];
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
	// Package managers keep Git data in writable caches. Only treat `.git` as
	// protected repository control data when it is inside the active workspace.
	if (cwd) {
		const workspace = resolve(cwd);
		const lexicalPath = resolve(path);
		const actualWorkspace = canonicalize(workspace);
		const actualPath = canonicalize(path);
		if (!isInside(workspace, lexicalPath) && !isInside(actualWorkspace, actualPath)) {
			return undefined;
		}
	}
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
	const ipVersion = isIP(value);
	if (ipVersion === 4) return value;
	if (ipVersion === 6) {
		const canonical = new URL(`http://[${value}]/`).hostname;
		return canonical.slice(1, -1).toLowerCase();
	}
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

export function permissionCoversPath(permission: IoPermission, path: string): boolean {
	const root = canonicalize(permission.path);
	const target = canonicalize(path);
	return permission.directory ? isInside(root, target) : root === target;
}

export function isDefaultWritePath(
	path: string,
	cwd: string,
	developmentCache?: DevelopmentCacheConfig,
): boolean {
	const actual = canonicalize(path);
	return (
		developmentCacheRightForPath(actual, developmentCache) !== undefined ||
		[canonicalize(cwd), canonicalize("/tmp"), canonicalize(tmpdir())].some((root) =>
			isInside(root, actual),
		)
	);
}
