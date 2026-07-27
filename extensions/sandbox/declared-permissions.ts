import type { CodexSandboxConfig } from "./codex-command.ts";
import {
	gitControlRoot,
	isControlRootSymlink,
	type IoPermission,
	normalizePermission,
	permissionCoversPath,
	projectControlRoot,
	resolveLexicalPermissionPath,
} from "./io-permissions.ts";
import {
	isBaseReadAllowed,
	isBaseWriteAllowed,
	isDeniedByConfig,
} from "./io-policy.ts";

export interface DeclaredFilesystemPermission {
	kind: "read" | "write";
	path: string;
	targetType?: "file" | "folder";
	reason: string;
}

export type FilePermission = Extract<IoPermission, { kind: "read" | "write" }>;

export interface CheckedDeclaredPermission {
	permission: FilePermission;
	reason: string;
	alreadyAllowed: boolean;
}

export function checkDeclaredFilesystemPermissions(
	declarations: readonly DeclaredFilesystemPermission[] | undefined,
	cwd: string,
	config: CodexSandboxConfig,
	persistentPermissions: readonly IoPermission[],
): CheckedDeclaredPermission[] {
	if ((declarations?.length ?? 0) > 16) {
		throw new Error("A command may declare at most 16 filesystem rights");
	}
	const checked = new Map<string, CheckedDeclaredPermission>();
	for (const declaration of declarations ?? []) {
		let path = declaration.path;
		let targetType = declaration.targetType;
		if (declaration.kind === "write") {
			const lexicalPath = resolveLexicalPermissionPath(path, cwd);
			const controlRoot =
				gitControlRoot(lexicalPath, cwd) ?? projectControlRoot(lexicalPath, cwd);
			if (controlRoot) {
				if (isControlRootSymlink(controlRoot)) {
					throw new Error(
						`Writes to a symlinked control folder cannot be granted: ${controlRoot}`,
					);
				}
				path = controlRoot;
				targetType = "folder";
			}
		}
		const permission = normalizePermission(
			{
				kind: declaration.kind,
				path,
				targetType,
			},
			cwd,
		) as FilePermission;
		if (isDeniedByConfig(permission.path, permission.kind, config, cwd)) {
			throw new Error(
				`Sandbox policy denies ${permission.kind} access to ${permission.path}`,
			);
		}
		const alreadyAllowed =
			persistentPermissions.some(
				(entry) =>
					entry.kind === permission.kind &&
					permissionCoversPath(entry, permission.path),
			) ||
			(permission.kind === "read"
				? isBaseReadAllowed(permission.path, config, cwd)
				: isBaseWriteAllowed(permission.path, config, cwd));
		checked.set(`${permission.kind}:${permission.path}`, {
			permission,
			reason: declaration.reason,
			alreadyAllowed,
		});
	}
	return [...checked.values()];
}
