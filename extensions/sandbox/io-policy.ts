import { homedir } from "node:os";
import { basename, isAbsolute, relative, resolve, sep } from "node:path";
import type { NativeSandboxConfig } from "./sandbox-config.ts";
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
	config: NativeSandboxConfig,
	cwd: string,
): boolean {
	const actual = canonicalize(path);
	return (config.filesystem?.allowRead ?? []).some((root) => {
		if (root === ":root") return true;
		if (root === ":workspace_roots" || root === ".") {
			return isInside(canonicalize(cwd), actual);
		}
		if (root.startsWith(":")) return false;
		return isInside(resolvePermissionPath(root, cwd), actual);
	});
}

export function isBaseWriteAllowed(
	path: string,
	config: NativeSandboxConfig,
	cwd: string,
): boolean {
	if (gitControlRoot(path, cwd)) return false;
	if (projectControlRoot(path, cwd)) return false;
	const actual = canonicalize(path);
	if (isDefaultWritePath(actual, cwd, config.developmentCache)) return true;
	return (config.filesystem?.allowWrite ?? []).some((root) => {
		if (root === "." || root === ":workspace_roots") {
			return isInside(canonicalize(cwd), actual);
		}
		if (root === ":tmpdir" || root === ":slash_tmp") {
			return isDefaultWritePath(actual, cwd, config.developmentCache);
		}
		if (root.startsWith(":")) return false;
		return isInside(resolvePermissionPath(root, cwd), actual);
	});
}

export function isDeniedByConfig(
	path: string,
	access: "read" | "write",
	config: NativeSandboxConfig,
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
		return isInside(resolveRulePath(rule, cwd), path);
	}
	const actualPath = canonicalize(path);
	const normalizedPath = actualPath.split(sep).join("/");
	const expandedRule = rule.startsWith("~/")
		? `${homedir()}/${rule.slice(2)}`
		: rule;
	const normalizedRule = expandedRule.split(sep).join("/");
	if (isAbsolute(expandedRule)) {
		return globRegex(normalizedRule).test(normalizedPath);
	}
	const relativePath = relative(canonicalize(cwd), actualPath).split(sep).join("/");
	if (relativePath === ".." || relativePath.startsWith("../")) return false;
	const pattern = globRegex(normalizedRule);
	return (
		pattern.test(relativePath) ||
		(!normalizedRule.includes("/") && pattern.test(basename(actualPath)))
	);
}

function resolveRulePath(rule: string, cwd: string): string {
	try {
		return resolvePermissionPath(rule, cwd);
	} catch (error) {
		if (
			error &&
			typeof error === "object" &&
			"code" in error &&
			(error.code === "EACCES" || error.code === "EPERM")
		) {
			if (rule === "~") return homedir();
			if (rule.startsWith("~/")) return resolve(homedir(), rule.slice(2));
			return isAbsolute(rule) ? resolve(rule) : resolve(cwd, rule);
		}
		throw error;
	}
}

function containsGlob(value: string): boolean {
	return value.includes("*") || value.includes("?") || value.includes("[");
}

function globRegex(glob: string): RegExp {
	let pattern = "";
	for (let index = 0; index < glob.length; index += 1) {
		const char = glob[index] ?? "";
		if (char === "*" && glob[index + 1] === "*") {
			if (glob[index + 2] === "/") {
				pattern += "(?:.*/)?";
				index += 2;
			} else {
				pattern += ".*";
				index += 1;
			}
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
