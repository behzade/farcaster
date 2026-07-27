import { existsSync, statSync } from "node:fs";
import { isAbsolute } from "node:path";
import type { BrokerDenial } from "./broker-client.ts";
import type { CodexSandboxConfig } from "./codex-command.ts";
import {
	canonicalize,
	gitControlRoot,
	isControlRootSymlink,
	isInside,
	isProtectedPath,
	isProtectedWritePath,
	permissionCoversPath,
	projectControlRoot,
	resolveLexicalPermissionPath,
	type IoPermission,
} from "./io-permissions.ts";
import {
	isBaseReadAllowed,
	isBaseWriteAllowed,
	isDeniedByConfig,
} from "./io-policy.ts";

export type NativeFilePermission = Extract<IoPermission, { kind: "read" | "write" }>;

export type NativeDenialDecision =
	| { kind: "ignore" }
	| { kind: "unsafe" }
	| { kind: "permission"; permission: NativeFilePermission };

/** Maps one kernel denial hint to one exact, safe permission prompt. */
export function permissionForNativeDenial(
	denial: BrokerDenial,
	cwd: string,
	config: CodexSandboxConfig,
	grants: readonly NativeFilePermission[],
	blockedPaths: readonly string[] = [],
): NativeDenialDecision {
	const access = denial.operation.startsWith("file-write")
		? "write"
		: denial.operation.startsWith("file-read")
			? "read"
			: undefined;
	if (!access || !denial.path || !isAbsolute(denial.path)) {
		return { kind: "ignore" };
	}

	try {
		const lexicalPath = resolveLexicalPermissionPath(denial.path, cwd);
		const path = canonicalize(lexicalPath);
		if (isInside("/dev", lexicalPath) || isInside(canonicalize("/dev"), path)) {
			return { kind: "ignore" };
		}
		if (blockedPaths.some((blocked) => canonicalize(blocked) === path)) {
			return { kind: "unsafe" };
		}
		const controlRoot =
			access === "write"
				? gitControlRoot(lexicalPath, cwd) ?? projectControlRoot(lexicalPath, cwd)
				: undefined;
		if (controlRoot && isControlRootSymlink(controlRoot)) return { kind: "unsafe" };
		const permissionPath = controlRoot ?? path;
		if (access === "read" && !existsSync(permissionPath)) return { kind: "ignore" };
		const directory =
			controlRoot !== undefined ||
			(existsSync(permissionPath) && statSync(permissionPath).isDirectory());

		const baseAllowed =
			access === "read"
				? isBaseReadAllowed(path, config, cwd) || isBaseWriteAllowed(path, config, cwd)
				: isBaseWriteAllowed(path, config, cwd);
		const alreadyGranted = grants.some(
			(permission) =>
				(permission.kind === access || (access === "read" && permission.kind === "write")) &&
				permissionCoversPath(permission, path),
		);
		if (baseAllowed || alreadyGranted) return { kind: "ignore" };
		if (
			isProtectedPath(path) ||
			(access === "write" && isProtectedWritePath(path)) ||
			isDeniedByConfig(path, access, config, cwd)
		) {
			return { kind: "unsafe" };
		}

		return {
			kind: "permission",
			permission: {
				kind: access,
				path: permissionPath,
				directory,
			},
		};
	} catch {
		return { kind: "unsafe" };
	}
}
