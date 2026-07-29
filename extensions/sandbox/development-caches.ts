import { existsSync, lstatSync, mkdirSync, realpathSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, isAbsolute, join, normalize, relative, resolve, sep } from "node:path";

export interface DevelopmentCacheWriteRight {
	path: string;
	directory: boolean;
}

export interface DevelopmentCacheConfig {
	root?: string;
	environment?: Record<string, string>;
}

export const DEFAULT_DEVELOPMENT_CACHE_CONFIG: Required<DevelopmentCacheConfig> = {
	root: "~/.cache/pi-sandbox",
	environment: {
		BUN_INSTALL_CACHE_DIR: "bun",
		CARGO_HOME: "cargo",
		CCACHE_DIR: "ccache",
		COREPACK_HOME: "corepack",
		DENO_DIR: "deno",
		GOCACHE: "go/build",
		GOMODCACHE: "go/mod",
		GRADLE_USER_HOME: "gradle",
		PIP_CACHE_DIR: "pip",
		POETRY_CACHE_DIR: "poetry",
		SCCACHE_DIR: "sccache",
		UV_CACHE_DIR: "uv",
		XDG_CACHE_HOME: "xdg",
		YARN_CACHE_FOLDER: "yarn",
		npm_config_cache: "npm",
	},
};

const ENV_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;

export function normalizeDevelopmentCacheConfig(
	value: unknown,
): DevelopmentCacheConfig | undefined {
	if (value === undefined) return undefined;
	if (!value || typeof value !== "object" || Array.isArray(value)) {
		throw new Error("developmentCache must be a JSON object");
	}
	const input = value as Record<string, unknown>;
	const unknown = Object.keys(input).filter(
		(key) => key !== "root" && key !== "environment",
	);
	if (unknown.length > 0) {
		throw new Error(`developmentCache contains unknown fields: ${unknown.join(", ")}`);
	}
	const root =
		input.root === undefined
			? undefined
			: validatedHomeRelativePath(input.root, "developmentCache.root", true);
	let environment: Record<string, string> | undefined;
	if (input.environment !== undefined) {
		if (
			!input.environment ||
			typeof input.environment !== "object" ||
			Array.isArray(input.environment)
		) {
			throw new Error("developmentCache.environment must be a JSON object");
		}
		environment = Object.fromEntries(
			Object.entries(input.environment).map(([name, path]) => {
				if (!ENV_NAME.test(name) || typeof path !== "string") {
					throw new Error(
						"developmentCache.environment must map valid environment names to relative paths",
					);
				}
				return [
					name,
					validatedHomeRelativePath(
						path,
						`developmentCache.environment.${name}`,
						false,
					),
				];
			}),
		);
	}
	return { root, environment };
}

export function developmentCacheRoot(
	config: DevelopmentCacheConfig = DEFAULT_DEVELOPMENT_CACHE_CONFIG,
	home = homedir(),
): string {
	const canonicalHome = existsSync(home) ? realpathSync.native(home) : resolve(home);
	const configuredRoot = config.root ?? DEFAULT_DEVELOPMENT_CACHE_CONFIG.root;
	const relativeRoot = configuredRoot.startsWith("~/")
		? configuredRoot.slice(2)
		: configuredRoot;
	const root = resolve(canonicalHome, relativeRoot);
	if (!isStrictDescendant(canonicalHome, root)) {
		throw new Error("developmentCache.root must resolve beneath the home directory");
	}
	return root;
}

/** Creates the sandbox-owned cache namespace from the trusted host. */
export function ensureDevelopmentCacheDirectories(
	config: DevelopmentCacheConfig = DEFAULT_DEVELOPMENT_CACHE_CONFIG,
	home = homedir(),
): void {
	const canonicalHome = existsSync(home) ? realpathSync.native(home) : resolve(home);
	const root = developmentCacheRoot(config, canonicalHome);
	ensureDirectoryTree(canonicalHome, root);
	if (!existsSync(root) || !statSync(root).isDirectory()) return;
	for (const path of new Set(Object.values(cacheEnvironmentForRoot(root, config)))) {
		ensureDirectoryTree(canonicalHome, path);
	}
}

/** Returns one writable cache namespace, omitting roots reached through symlinks. */
export function developmentCacheWriteRights(
	config: DevelopmentCacheConfig = DEFAULT_DEVELOPMENT_CACHE_CONFIG,
	home = homedir(),
): DevelopmentCacheWriteRight[] {
	const canonicalHome = existsSync(home) ? realpathSync.native(home) : resolve(home);
	const root = developmentCacheRoot(config, canonicalHome);
	if (hasSymlinkBelow(canonicalHome, root)) return [];
	if (!existsSync(root) && !existsSync(dirname(root))) return [];
	if (existsSync(root) && !statSync(root).isDirectory()) return [];
	return [
		{
			path: existsSync(root) ? realpathSync.native(root) : root,
			directory: true,
		},
	];
}

export function developmentCacheWriteRightsForWorkspace(
	workspace: string,
	config: DevelopmentCacheConfig = DEFAULT_DEVELOPMENT_CACHE_CONFIG,
	home = homedir(),
): DevelopmentCacheWriteRight[] {
	const actualWorkspace = canonicalPath(workspace);
	return developmentCacheWriteRights(config, home).filter(
		(right) =>
			!isPathInside(actualWorkspace, right.path) &&
			!isPathInside(right.path, actualWorkspace),
	);
}

export function developmentCacheRightForPath(
	path: string,
	config: DevelopmentCacheConfig = DEFAULT_DEVELOPMENT_CACHE_CONFIG,
	home = homedir(),
): DevelopmentCacheWriteRight | undefined {
	const target = canonicalPath(path);
	return developmentCacheWriteRights(config, home).find((right) =>
		isPathInside(right.path, target),
	);
}

/** Redirects tool caches into the sandbox-owned writable namespace. */
export function developmentCacheEnvironment(
	config: DevelopmentCacheConfig = DEFAULT_DEVELOPMENT_CACHE_CONFIG,
	home = homedir(),
): Record<string, string> {
	const root = developmentCacheWriteRights(config, home)[0]?.path;
	if (!root) return {};
	return cacheEnvironmentForRoot(root, config);
}

function cacheEnvironmentForRoot(
	root: string,
	config: DevelopmentCacheConfig,
): Record<string, string> {
	const configuredEnvironment = {
		...DEFAULT_DEVELOPMENT_CACHE_CONFIG.environment,
		...(config.environment ?? {}),
	};
	return Object.fromEntries(
		Object.entries(configuredEnvironment).map(([name, path]) => {
			if (!ENV_NAME.test(name)) {
				throw new Error(`invalid development-cache environment name: ${name}`);
			}
			const target = resolve(root, validatedHomeRelativePath(path, name, false));
			if (!isStrictDescendant(root, target)) {
				throw new Error(`development-cache environment path escapes its root: ${name}`);
			}
			return [name, target];
		}),
	);
}

function isPathInside(root: string, path: string): boolean {
	const rel = relative(root, path);
	return rel === "" || (rel !== ".." && !rel.startsWith(`..${sep}`));
}

function isStrictDescendant(root: string, path: string): boolean {
	return path !== root && isPathInside(root, path);
}

function validatedHomeRelativePath(
	value: unknown,
	field: string,
	allowHomePrefix: boolean,
): string {
	if (typeof value !== "string" || value.length === 0 || value.includes("\0")) {
		throw new Error(`${field} must be a non-empty relative path`);
	}
	const relativePath = allowHomePrefix && value.startsWith("~/") ? value.slice(2) : value;
	const normalized = normalize(relativePath);
	if (
		isAbsolute(relativePath) ||
		normalized === "." ||
		normalized === ".." ||
		normalized.startsWith(`..${sep}`)
	) {
		throw new Error(`${field} must be a non-empty relative path beneath its root`);
	}
	return normalized;
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
		// An unsafe or unavailable cache root receives no implicit write right.
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
