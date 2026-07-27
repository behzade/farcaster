import { basename, relative, sep } from "node:path";
import type { CodexSandboxConfig } from "./codex-command.ts";
import {
	canonicalize,
	gitControlRoot,
	isDefaultWritePath,
	isInside,
	projectControlRoot,
	resolvePermissionPath,
} from "./io-permissions.ts";

export function isBaseReadAllowed(
	path: string,
	config: CodexSandboxConfig,
	cwd: string,
): boolean {
	return (config.filesystem?.allowRead ?? []).some((root) => {
		if (root === ":root") return true;
		if (root === ":workspace_roots" || root === ".") {
			return isInside(canonicalize(cwd), path);
		}
		if (root.startsWith(":")) return false;
		return isInside(resolvePermissionPath(root, cwd), path);
	});
}

export function isBaseWriteAllowed(
	path: string,
	config: CodexSandboxConfig,
	cwd: string,
): boolean {
	if (gitControlRoot(path)) return false;
	if (projectControlRoot(path, cwd)) return false;
	if (isDefaultWritePath(path, cwd)) return true;
	return (config.filesystem?.allowWrite ?? []).some((root) => {
		if (root === "." || root === ":workspace_roots") {
			return isInside(canonicalize(cwd), path);
		}
		if (root === ":tmpdir" || root === ":slash_tmp") {
			return isDefaultWritePath(path, cwd);
		}
		if (root.startsWith(":")) return false;
		return isInside(resolvePermissionPath(root, cwd), path);
	});
}

export function isDeniedByConfig(
	path: string,
	access: "read" | "write",
	config: CodexSandboxConfig,
	cwd: string,
): boolean {
	const rules = [
		...(config.filesystem?.denyRead ?? []),
		...(access === "write" ? (config.filesystem?.denyWrite ?? []) : []),
	];
	return rules.some((rule) => matchesPathRule(rule, path, cwd));
}

export function matchesPathRule(rule: string, path: string, cwd: string): boolean {
	if (rule.startsWith(":")) return false;
	if (!containsGlob(rule)) {
		return isInside(resolvePermissionPath(rule, cwd), path);
	}
	const normalizedPath = path.split(sep).join("/");
	const relativePath = relative(cwd, path).split(sep).join("/");
	const pattern = globRegex(rule.split(sep).join("/"));
	return (
		pattern.test(normalizedPath) ||
		pattern.test(relativePath) ||
		pattern.test(basename(path))
	);
}

function containsGlob(value: string): boolean {
	return value.includes("*") || value.includes("?") || value.includes("[");
}

function globRegex(glob: string): RegExp {
	let pattern = "";
	for (let index = 0; index < glob.length; index += 1) {
		const char = glob[index] ?? "";
		if (char === "*" && glob[index + 1] === "*") {
			pattern += ".*";
			index += 1;
		} else if (char === "*") {
			pattern += "[^/]*";
		} else if (char === "?") {
			pattern += "[^/]";
		} else {
			pattern += char.replace(/[\\^$.*+?()[\]{}|]/g, "\\$&");
		}
	}
	return new RegExp(`^(?:${pattern})$`);
}
