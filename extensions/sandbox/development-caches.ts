import { existsSync, lstatSync, mkdirSync, realpathSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, relative, resolve, sep } from "node:path";

export interface DevelopmentCacheWriteRight {
	path: string;
	directory: boolean;
}

interface CacheDefinition {
	path: string;
	directory: boolean;
}

const COMMON_CACHES: readonly CacheDefinition[] = [
	{ path: ".cargo/registry", directory: true },
	{ path: ".cargo/git", directory: true },
	{ path: ".cargo/.package-cache", directory: false },
	{ path: ".cargo/.package-cache-mutate", directory: false },
	{ path: ".cargo/.global-cache", directory: false },
	{ path: ".cargo/.global-cache-journal", directory: false },
	{ path: ".cargo/.global-cache-shm", directory: false },
	{ path: ".cargo/.global-cache-wal", directory: false },
	{ path: ".npm", directory: true },
	{ path: ".bun/install/cache", directory: true },
	{ path: ".yarn/berry", directory: true },
	{ path: ".cache/uv", directory: true },
	{ path: "go/pkg/mod", directory: true },
	{ path: "go/pkg/sumdb", directory: true },
];

const DARWIN_CACHES: readonly CacheDefinition[] = [
	{ path: "Library/Caches/deno", directory: true },
	{ path: "Library/Caches/go-build", directory: true },
	{ path: "Library/Caches/node", directory: true },
	{ path: "Library/Caches/node-gyp", directory: true },
	{ path: "Library/Caches/pip", directory: true },
	{ path: "Library/Caches/pnpm", directory: true },
	{ path: "Library/Caches/Yarn", directory: true },
	{ path: "Library/pnpm/store", directory: true },
];

const LINUX_CACHES: readonly CacheDefinition[] = [
	{ path: ".cache/deno", directory: true },
	{ path: ".cache/go-build", directory: true },
	{ path: ".cache/node", directory: true },
	{ path: ".cache/node-gyp", directory: true },
	{ path: ".cache/pip", directory: true },
	{ path: ".cache/pnpm", directory: true },
	{ path: ".cache/yarn", directory: true },
	{ path: ".local/share/pnpm/store", directory: true },
];

/** Creates fixed cache directories from the trusted host before sandbox launch. */
export function ensureDevelopmentCacheDirectories(
	home = homedir(),
	platform = process.platform,
): void {
	const canonicalHome = existsSync(home) ? realpathSync.native(home) : resolve(home);
	const platformCaches =
		platform === "darwin" ? DARWIN_CACHES : platform === "linux" ? LINUX_CACHES : [];
	for (const definition of [...COMMON_CACHES, ...platformCaches]) {
		const target = join(
			canonicalHome,
			definition.directory ? definition.path : dirname(definition.path),
		);
		ensureDirectoryTree(canonicalHome, target);
	}
}

/** Returns fixed package/build cache rights, omitting roots reached through symlinks. */
export function developmentCacheWriteRights(
	home = homedir(),
	platform = process.platform,
): DevelopmentCacheWriteRight[] {
	const canonicalHome = existsSync(home) ? realpathSync.native(home) : resolve(home);
	const platformCaches =
		platform === "darwin" ? DARWIN_CACHES : platform === "linux" ? LINUX_CACHES : [];
	const rights: DevelopmentCacheWriteRight[] = [];
	for (const definition of [...COMMON_CACHES, ...platformCaches]) {
		const path = join(canonicalHome, definition.path);
		if (hasSymlinkBelow(canonicalHome, path)) continue;
		if (!existsSync(path) && !existsSync(dirname(path))) continue;
		if (existsSync(path) && statSync(path).isDirectory() !== definition.directory) continue;
		rights.push({
			path: existsSync(path) ? realpathSync.native(path) : path,
			directory: definition.directory,
		});
	}
	return rights;
}

export function developmentCacheWriteRightsForWorkspace(
	workspace: string,
	home = homedir(),
	platform = process.platform,
): DevelopmentCacheWriteRight[] {
	const actualWorkspace = canonicalPath(workspace);
	return developmentCacheWriteRights(home, platform).filter(
		(right) =>
			!isPathInside(actualWorkspace, right.path) &&
			!isPathInside(right.path, actualWorkspace),
	);
}

export function developmentCacheRightForPath(
	path: string,
	home = homedir(),
	platform = process.platform,
): DevelopmentCacheWriteRight | undefined {
	const target = canonicalPath(path);
	return developmentCacheWriteRights(home, platform).find((right) =>
		right.directory ? isPathInside(right.path, target) : target === right.path,
	);
}

function isPathInside(root: string, path: string): boolean {
	const rel = relative(root, path);
	return rel === "" || (rel !== ".." && !rel.startsWith(`..${sep}`));
}

function ensureDirectoryTree(root: string, target: string): void {
	const rel = relative(root, target);
	if (rel === "" || rel === ".." || rel.startsWith(`..${sep}`)) return;
	let current = root;
	try {
		for (const part of rel.split(sep)) {
			current = join(current, part);
			if (existsSync(current)) {
				const metadata = lstatSync(current);
				if (metadata.isSymbolicLink() || !metadata.isDirectory()) return;
				continue;
			}
			mkdirSync(current, { mode: 0o700 });
		}
	} catch {
		// A cache that cannot be prepared receives no useful implicit right.
	}
}

function canonicalPath(path: string): string {
	const absolute = resolve(path);
	if (existsSync(absolute)) return realpathSync.native(absolute);
	const parent = resolve(absolute, "..");
	if (parent === absolute) return absolute;
	return join(canonicalPath(parent), relative(parent, absolute));
}

function hasSymlinkBelow(root: string, path: string): boolean {
	const rel = relative(root, path);
	if (rel === "" || rel === ".." || rel.startsWith(`..${sep}`)) return true;
	let current = root;
	for (const part of rel.split(sep)) {
		current = join(current, part);
		try {
			if (lstatSync(current).isSymbolicLink()) return true;
		} catch (error) {
			if (isMissing(error)) return false;
			return true;
		}
	}
	return false;
}

function isMissing(error: unknown): boolean {
	return Boolean(
		error &&
			typeof error === "object" &&
			"code" in error &&
			(error as { code?: unknown }).code === "ENOENT",
	);
}
