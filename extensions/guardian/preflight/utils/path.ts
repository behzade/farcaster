import { homedir } from "node:os";
import { isAbsolute, resolve, sep, join } from "node:path";

export function expandTilde(value: string): string {
	if (value === "~") return homedir();
	if (value.startsWith("~/")) {
		return join(homedir(), value.slice(2));
	}
	return value;
}

export function resolveScope(scope: string, cwd: string): string | undefined {
	if (!scope) return undefined;
	const expanded = expandTilde(scope);
	const resolved = isAbsolute(expanded) ? resolve(expanded) : resolve(cwd, expanded);
	return resolved;
}

export function isPathWithin(targetPath: string, basePath: string): boolean {
	const normalizedTarget = resolve(targetPath);
	const normalizedBase = resolve(basePath);
	if (normalizedTarget === normalizedBase) return true;
	return normalizedTarget.startsWith(`${normalizedBase}${sep}`);
}

export function isScopeOutsideWorkspace(scopes: string[], cwd: string): boolean {
	const basePath = resolve(cwd);
	for (const scope of scopes) {
		const resolvedScope = resolveScope(scope, cwd);
		if (!resolvedScope) continue;
		if (!isPathWithin(resolvedScope, basePath)) {
			return true;
		}
	}
	return false;
}

export function toPosixPath(value: string): string {
	return value.split(sep).join("/");
}
