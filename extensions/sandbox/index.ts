/**
 * Pi sandbox and IO permission broker.
 *
 * Commands may use any interpreter or child process. The broker applies the same
 * filesystem and network profile to the whole process tree. Filesystem rights
 * are derived from sandbox denials; the model may not declare them.
 */

import { randomUUID } from "node:crypto";
import { existsSync, readFileSync, statSync } from "node:fs";
import { basename, dirname, resolve } from "node:path";
import type {
	ExtensionAPI,
	ExtensionContext,
	ToolCallEvent,
} from "@earendil-works/pi-coding-agent";
import {
	type BashOperations,
	CONFIG_DIR_NAME,
	createBashTool,
	getAgentDir,
} from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { SandboxBrokerClient } from "./broker-client.ts";
import {
	isValidBackgroundJobName,
	modelVisibleBackgroundJobOutput,
} from "./background-jobs.ts";
import {
	developmentCacheRoot,
	ensureDevelopmentCacheDirectories,
} from "./development-caches.ts";
import {
	DEFAULT_CONFIG,
	applyProjectRestrictions,
	type NativeSandboxConfig,
	type NativeSandboxGrants,
	mergeGlobalConfig,
	normalizeConfig,
} from "./sandbox-config.ts";
import {
	canonicalize,
	gitControlRoot,
	grantsToRuntime,
	type IoPermission,
	isInside,
	isControlRootSymlink,
	isProtectedPath,
	isProtectedWritePath,
	loadWorkspacePermissions,
	permissionCoversPath,
	permissionLabel,
	projectControlRoot,
	normalizePermission,
	resolveLexicalPermissionPath,
	resolvePermissionPath,
	saveWorkspacePermission,
} from "./io-permissions.ts";
import {
	isBaseReadAllowed,
	isBaseWriteAllowed,
	isDeniedByConfig,
} from "./io-policy.ts";
import {
	createApprovingNativeSandboxOps,
	createNativeSandboxOps,
	modelVisibleApprovedRetryOutput,
	type NativeApprovalChoice,
	type NativeApprovalRequest,
	resolveNativeApprovalChoice,
} from "./native-sandbox-ops.ts";
import { requestUserApproval } from "./permission-system-approval.ts";
import { backgroundKeyBytes, NativeBackgroundJobs } from "./native-background-jobs.ts";

function readConfig(path: string): NativeSandboxConfig | undefined {
	if (!existsSync(path)) return undefined;
	const parsed: unknown = JSON.parse(readFileSync(path, "utf8"));
	return normalizeConfig(parsed);
}

function loadConfig(cwd: string, projectTrusted: boolean): NativeSandboxConfig {
	const globalPath = resolve(getAgentDir(), "extensions", "sandbox.json");
	const projectPath = resolve(cwd, CONFIG_DIR_NAME, "sandbox.json");
	const global = readConfig(globalPath) ?? {};
	const base = mergeGlobalConfig(DEFAULT_CONFIG, global);
	if (!projectTrusted) return base;
	return applyProjectRestrictions(base, readConfig(projectPath) ?? {});
}

const PACKAGED_BROKER_PATH = "@PI_SANDBOX_BROKER@/bin/pi-sandbox-broker";

function unavailableBashOps(reason: string): BashOperations {
	return {
		async exec() {
			throw new Error(reason);
		},
	};
}

type SandboxState =
	| { kind: "disabled"; reason: string }
	| { kind: "initializing" }
	| { kind: "ready"; config: NativeSandboxConfig }
	| { kind: "failed"; reason: string };

type NetworkPermission = Extract<IoPermission, { kind: "network_host" }>;
type FilePermission = Extract<IoPermission, { kind: "read" | "write" }>;
type DeclaredNetworkPermission = {
	kind: "network_host";
	host: string;
	reason: string;
};

function isSafeSavedFilePermission(permission: IoPermission): permission is FilePermission {
	return (
		(permission.kind === "read" || permission.kind === "write") &&
		!isProtectedPath(permission.path) &&
		(permission.kind !== "write" || !isProtectedWritePath(permission.path)) &&
		!isControlRootSymlink(permission.path)
	);
}

const NetworkPermissionParams = Type.Object({
	host: Type.String({
		description: "One exact hostname or IP, with no scheme, port, path, or wildcard",
	}),
	reason: Type.String({ description: "Why this host is needed" }),
});

const DeclaredNetworkPermissionParams = Type.Object(
	{
		kind: Type.Literal("network_host"),
		host: Type.String({
			description: "One exact hostname or IP, with no scheme, port, path, or wildcard",
		}),
		reason: Type.String({ description: "Why this command needs the host" }),
	},
	{ additionalProperties: false },
);

const BashParams = Type.Object({
	command: Type.String({ description: "Bash command to execute" }),
	timeout: Type.Optional(
		Type.Number({ description: "Timeout in seconds (optional, no default timeout)" }),
	),
	permissions: Type.Optional(
		Type.Array(DeclaredNetworkPermissionParams, { maxItems: 16 }),
	),
});

const BackgroundJobParams = Type.Union([
	Type.Object({
		action: Type.Literal("start"),
		name: Type.String({ description: "Unique job name starting with pi-" }),
		command: Type.String({ description: "Shell command to run in the background" }),
		cwd: Type.Optional(Type.String({ description: "Working directory inside this workspace" })),
		permissions: Type.Optional(
			Type.Array(DeclaredNetworkPermissionParams, { maxItems: 16 }),
		),
	}),
	Type.Object({ action: Type.Literal("list") }),
	Type.Object({
		action: Type.Literal("status"),
		name: Type.String({ description: "Job name" }),
	}),
	Type.Object({
		action: Type.Literal("read"),
		name: Type.String({ description: "Job name" }),
		lines: Type.Optional(Type.Integer({ minimum: 1, maximum: 10_000 })),
	}),
	Type.Object({
		action: Type.Literal("write"),
		name: Type.String({ description: "Job name" }),
		text: Type.String({ description: "Text to send without Enter" }),
	}),
	Type.Object({
		action: Type.Literal("line"),
		name: Type.String({ description: "Job name" }),
		text: Type.String({ description: "Text to send followed by Enter" }),
	}),
	Type.Object({
		action: Type.Literal("keys"),
		name: Type.String({ description: "Job name" }),
		keys: Type.Array(Type.String(), { minItems: 1, maxItems: 20 }),
	}),
	Type.Object({
		action: Type.Literal("stop"),
		name: Type.String({ description: "Job name" }),
	}),
]);

interface ApprovalDecision {
	allowed: boolean;
	persistent: boolean;
	reason?: string;
}

function networkDeclarations(
	permissions: readonly DeclaredNetworkPermission[] | undefined,
): DeclaredNetworkPermission[] {
	return [...(permissions ?? [])];
}

function networkRuleMatches(rule: string, host: string): boolean {
	const normalized = rule.toLowerCase();
	if (normalized === "*") return true;
	if (normalized.startsWith("**.")) {
		const base = normalized.slice(3);
		return host === base || host.endsWith(`.${base}`);
	}
	if (normalized.startsWith("*.")) {
		const base = normalized.slice(2);
		return host !== base && host.endsWith(`.${base}`);
	}
	return normalized === host;
}

export default function (pi: ExtensionAPI) {
	pi.registerFlag("no-sandbox", {
		description: "Disable OS-level sandboxing for bash commands",
		type: "boolean",
		default: false,
	});

	const localCwd = process.cwd();
	const localBash = createBashTool(localCwd);
	const permissionFile = resolve(getAgentDir(), "io-permissions.json");
	let sandboxState: SandboxState = { kind: "initializing" };
	let persistentPermissions: IoPermission[] = [];
	let brokerClient: SandboxBrokerClient | undefined;
	let backgroundJobs: NativeBackgroundJobs | undefined;
	let userBashCounter = 0;
	let sessionGeneration = 0;

	const runtimeGrants = (): NativeSandboxGrants => {
		const grants = grantsToRuntime(persistentPermissions);
		return {
			read: grants.read,
			write: grants.write,
			networkHosts: grants.networkHosts,
		};
	};

	const promptForToolPermission = async (
		permission: IoPermission,
		tool: {
			toolName: string;
			toolCallId: string;
			reason?: string;
			retry?: boolean;
			allowOnce?: boolean;
			signal?: AbortSignal;
		},
		ctx: ExtensionContext,
	): Promise<ApprovalDecision> => {
		const label = permissionLabel(permission);
		pi.events.emit("approval:requested", {
			kind: "io-permission",
			title: "Tool requests an IO right",
			summary: `${tool.toolName} requests ${label}${
				tool.reason ? `\nReason: ${tool.reason}` : ""
			}`,
			toolName: tool.toolName,
			toolCallId: tool.toolCallId,
			sessionId: ctx.sessionManager.getSessionId(),
			cwd: ctx.cwd,
		});
		const allowOnce = tool.retry ? "Allow once and retry" : "Allow once";
		const allowAlways = tool.retry
			? "Always allow in this workspace and retry"
			: "Always allow in this workspace";
		const result = await requestUserApproval(ctx, {
			requestId: tool.toolCallId,
			title: "Tool requests an IO right",
			message: `Allow ${tool.toolName} to access ${label}?${
				tool.reason ? `\n\nReason: ${tool.reason}` : ""
			}`,
			source: "tool_call",
			surface: permission.kind,
			value: permission.path,
			choices: [
				...(tool.allowOnce === false
					? []
					: [{ id: "allow-once", label: allowOnce }]),
				{ id: "allow-always", label: allowAlways },
				{ id: "deny", label: "No" },
				{ id: "deny-with-comment", label: "No, with comment", requestReason: true },
			],
			reasonTitle: "Tell the agent what to do instead",
			reasonPlaceholder: "Short note",
			signal: tool.signal,
		});
		const allow = result.choiceId === "allow-once" || result.choiceId === "allow-always";
		if (result.choiceId === "allow-always") {
			saveWorkspacePermission(permissionFile, ctx.cwd, permission);
			persistentPermissions = loadWorkspacePermissions(permissionFile, ctx.cwd);
		}
		pi.events.emit("approval:resolved", {
			kind: "io-permission",
			toolName: tool.toolName,
			toolCallId: tool.toolCallId,
			decision: allow ? "allowed" : "denied",
		});
		if (allow) {
			return {
				allowed: true,
				persistent: result.choiceId === "allow-always",
			};
		}
		return {
			allowed: false,
			persistent: false,
			reason: result.reason
				? `Permission denied. User comment: ${result.reason}`
				: (result.unavailableReason ?? "Permission denied by user"),
		};
	};

	const promptForNativeApproval = async (
		request: NativeApprovalRequest,
		tool: { toolName: string; toolCallId: string; signal?: AbortSignal },
		ctx: ExtensionContext,
	): Promise<readonly FilePermission[] | undefined> => {
		const permissions = [...request.permissions];
		const folder = request.folderAlternative;
		if (!folder || permissions.length < 2) {
			const approved: FilePermission[] = [];
			for (const permission of permissions) {
				const decision = await promptForToolPermission(
					permission,
					{ ...tool, retry: true },
					ctx,
				);
				if (!decision.allowed) return undefined;
				approved.push(permission);
			}
			return approved;
		}

		const access = permissions[0]?.kind ?? folder.kind;
		const exactOnce = `Allow these ${permissions.length} files once and retry`;
		const exactAlways = `Always allow these ${permissions.length} files in this workspace and retry`;
		const folderOnce = `Allow ${folder.path} recursively once and retry`;
		const folderAlways = `Always allow ${folder.path} recursively in this workspace and retry`;
		const paths = permissions.map((permission) => `- ${permission.path}`).join("\n");
		const summary =
			`${tool.toolName} requests ${access} access to ${permissions.length} files in ${folder.path}. ` +
			`The folder choices grant recursive ${access} access.`;
		pi.events.emit("approval:requested", {
			kind: "io-permission",
			title: "Tool requests grouped IO rights",
			summary,
			toolName: tool.toolName,
			toolCallId: tool.toolCallId,
			sessionId: ctx.sessionManager.getSessionId(),
			cwd: ctx.cwd,
		});
		const result = await requestUserApproval(ctx, {
			requestId: tool.toolCallId,
			title: "Tool requests grouped IO rights",
			message: `${summary}\n\n${paths}`,
			source: "tool_call",
			surface: access,
			value: folder.path,
			choices: [
				{ id: "exact-once", label: exactOnce },
				{ id: "exact-always", label: exactAlways },
				{ id: "folder-once", label: folderOnce },
				{ id: "folder-always", label: folderAlways },
				{ id: "deny", label: "No" },
			],
			signal: tool.signal,
		});
		const choice: NativeApprovalChoice =
			result.choiceId === "exact-once" ||
			result.choiceId === "exact-always" ||
			result.choiceId === "folder-once" ||
			result.choiceId === "folder-always"
				? result.choiceId
				: "deny";
		const resolved = resolveNativeApprovalChoice(request, choice);
		if (resolved?.persistent) {
			for (const permission of resolved.permissions) {
				saveWorkspacePermission(permissionFile, ctx.cwd, permission);
			}
			persistentPermissions = loadWorkspacePermissions(permissionFile, ctx.cwd);
		}
		pi.events.emit("approval:resolved", {
			kind: "io-permission",
			toolName: tool.toolName,
			toolCallId: tool.toolCallId,
			decision: resolved ? "allowed" : "denied",
		});
		return resolved?.permissions;
	};

	const approveDeclaredNetworkPermissions = async (
		declarations: readonly DeclaredNetworkPermission[] | undefined,
		tool: { toolName: string; toolCallId: string },
		ctx: ExtensionContext,
		config: NativeSandboxConfig,
	): Promise<NetworkPermission[]> => {
		if ((declarations?.length ?? 0) > 16) {
			throw new Error("A command may declare at most 16 network hosts");
		}
		const grants = new Map<string, NetworkPermission>();
		for (const declaration of declarations ?? []) {
			const permission = normalizePermission(
				{ kind: "network_host", host: declaration.host },
				ctx.cwd,
			) as NetworkPermission;
			if (config.network?.enabled === false) {
				throw new Error("Network access is disabled by the sandbox policy");
			}
			if (
				(config.network?.deniedDomains ?? []).some((rule) =>
					networkRuleMatches(rule, permission.host),
				)
			) {
				throw new Error(`${permission.host} is denied by the sandbox policy`);
			}
			const alreadyAllowed = [
				...(config.network?.allowedDomains ?? []),
				...grantsToRuntime(persistentPermissions).networkHosts,
			].some((rule) => networkRuleMatches(rule, permission.host));
			if (!alreadyAllowed) {
				const decision = await promptForToolPermission(
					permission,
					{ ...tool, reason: declaration.reason, retry: false },
					ctx,
				);
				if (!decision.allowed) {
					throw new Error(decision.reason ?? "Permission denied by user");
				}
			}
			grants.set(permission.host, permission);
		}
		return [...grants.values()];
	};

	pi.registerTool({
		name: "request_network_permission",
		label: "Request network host",
		description:
			"Ask the user to save one exact hostname or IP for this workspace. For a one-command host right, declare network_host in that bash or background-job start's permissions. Do not include a scheme, port, path, or wildcard.",
		promptSnippet:
			"Declare a command-only network_host in bash permissions. Use request_network_permission only to save a host for this workspace.",
		parameters: NetworkPermissionParams,
		executionMode: "sequential",
		async execute(toolCallId, params, _signal, _onUpdate, ctx) {
			if (sandboxState.kind !== "ready") {
				return {
					content: [{ type: "text", text: "The sandbox is not ready, so no network host was granted." }],
					details: { granted: false, reason: "sandbox-not-ready" },
					isError: true,
				};
			}
			let permission: NetworkPermission;
			try {
				permission = normalizePermission(
					{ kind: "network_host", host: params.host },
					ctx.cwd,
				) as NetworkPermission;
			} catch (error) {
				return {
					content: [
						{ type: "text", text: error instanceof Error ? error.message : String(error) },
					],
					details: { granted: false, reason: "invalid-host" },
					isError: true,
				};
			}
			const config = sandboxState.config;
			if (config.network?.enabled === false) {
				return {
					content: [{ type: "text", text: "Network access is disabled by the sandbox policy." }],
					details: { granted: false, reason: "network-disabled" },
					isError: true,
				};
			}
			if (
				(config.network?.deniedDomains ?? []).some((rule) =>
					networkRuleMatches(rule, permission.host),
				)
			) {
				return {
					content: [{ type: "text", text: `${permission.host} is denied by the sandbox policy.` }],
					details: { granted: false, reason: "host-denied" },
					isError: true,
				};
			}
			const existing = [
				...(config.network?.allowedDomains ?? []),
				...grantsToRuntime(persistentPermissions).networkHosts,
			].some((rule) => networkRuleMatches(rule, permission.host));
			if (existing) {
				return {
					content: [{ type: "text", text: `${permission.host} is already allowed.` }],
					details: { granted: true, existing: true },
				};
			}
			const decision = await promptForToolPermission(
				permission,
				{
					toolName: "request_network_permission",
					toolCallId,
					reason: params.reason,
					allowOnce: false,
				},
				ctx,
			);
			return {
				content: [
					{
						type: "text",
						text: decision.allowed
							? `${permission.host} is allowed for this workspace.`
							: decision.reason ?? "Network host denied.",
					},
				],
				details: {
					granted: decision.allowed,
					scope: decision.allowed ? "workspace" : "none",
					host: permission.host,
				},
				isError: !decision.allowed,
			};
		},
	});

	pi.registerTool({
		name: "background_job",
		label: "Background job",
		description:
			"Start, list, inspect, interact with, or stop a session-scoped long-running command. Each job runs in its own native OS sandbox with job-scoped filesystem and exact-host network rights. Names must start with pi- and use only letters, digits, dots, underscores, or hyphens.",
		promptSnippet:
			"Use background_job for long-running servers, watchers, builds, and tests. Stop jobs created for the current task when they are no longer needed.",
		parameters: BackgroundJobParams,
		executionMode: "sequential",
		async execute(toolCallId, params, _signal, _onUpdate, ctx) {
			if ("name" in params && !isValidBackgroundJobName(params.name)) {
				return {
					content: [{ type: "text", text: "Job names must start with pi- and use only letters, digits, dots, underscores, or hyphens." }],
					isError: true,
				};
			}
			if (sandboxState.kind !== "ready" || !backgroundJobs) {
				return {
					content: [{ type: "text", text: "The native sandbox is not ready." }],
					isError: true,
				};
			}
			try {
				let output: string;
				if (params.action === "start") {
					const cwd = resolvePermissionPath(params.cwd ?? ctx.cwd, ctx.cwd);
					if (!isInside(canonicalize(ctx.cwd), cwd)) {
						throw new Error("Background jobs must start inside the current workspace.");
					}
					if (!existsSync(cwd) || !statSync(cwd).isDirectory()) {
						throw new Error(`Background job directory does not exist: ${cwd}`);
					}
					const declared = await approveDeclaredNetworkPermissions(
						networkDeclarations(params.permissions),
						{ toolName: "background_job", toolCallId },
						ctx,
						sandboxState.config,
					);
					const grants = runtimeGrants();
					const declaredGrants = grantsToRuntime(declared);
					output = await backgroundJobs.start({
						name: params.name,
						command: params.command,
						cwd,
						config: sandboxState.config,
						permissions: persistentPermissions.filter(isSafeSavedFilePermission),
						networkHosts: [...(grants.networkHosts ?? []), ...declaredGrants.networkHosts],
					});
				} else if (params.action === "list") {
					output = backgroundJobs.list();
				} else if (params.action === "status") {
					output = backgroundJobs.status(params.name);
				} else if (params.action === "read") {
					output = modelVisibleBackgroundJobOutput(
						"read",
						backgroundJobs.read(params.name, params.lines ?? 200),
					);
				} else if (params.action === "write") {
					output = backgroundJobs.write(params.name, Buffer.from(params.text));
				} else if (params.action === "line") {
					output = backgroundJobs.write(params.name, Buffer.from(`${params.text}\n`));
				} else if (params.action === "keys") {
					output = backgroundJobs.write(params.name, backgroundKeyBytes(params.keys));
				} else {
					output = await backgroundJobs.stop(params.name);
				}
				return { content: [{ type: "text", text: output || "Done" }], details: { action: params.action } };
			} catch (error) {
				return {
					content: [{ type: "text", text: error instanceof Error ? error.message : String(error) }],
					isError: true,
				};
			}
		},
	});

	pi.registerTool({
		...localBash,
		label: "bash (OS sandbox)",
		description:
			"Execute a bash command in the OS sandbox. Filesystem access is inferred from sandbox denials; do not predict or declare filesystem rights. The current workspace, temp folders, and the sandbox-owned development-cache namespace already have their needed rights. Only exact network_host rights may be declared in permissions.",
		promptSnippet:
			"Do not declare filesystem permissions. Run the command and let the sandbox identify any denied write. Use permissions only for exact network hosts. Treat time blocked on permission approval as permission wait; never report it as no stall if the command did not start.",
		parameters: BashParams,
		executionMode: "sequential",
		renderShell: "self",
		async execute(id, params, signal, onUpdate, ctx) {
			if (sandboxState.kind === "disabled") {
				return localBash.execute(id, params, signal, onUpdate);
			}
			if (sandboxState.kind !== "ready") {
				throw new Error(
					sandboxState.kind === "failed"
						? sandboxState.reason
						: "Sandbox is still initializing; command blocked",
				);
			}
			const declaredNetworkPermissions = await approveDeclaredNetworkPermissions(
				networkDeclarations(params.permissions),
				{ toolName: "bash", toolCallId: id },
				ctx,
				sandboxState.config,
			);
			const grants = runtimeGrants();
			const declaredGrants = grantsToRuntime(declaredNetworkPermissions);
			grants.networkHosts = [
				...(grants.networkHosts ?? []),
				...declaredGrants.networkHosts,
			];
			if (!brokerClient) throw new Error("Native sandbox broker is not ready");
			const filePermissions = persistentPermissions.filter(isSafeSavedFilePermission);
			const operations = createApprovingNativeSandboxOps({
				client: brokerClient,
				config: sandboxState.config,
				initialPermissions: filePermissions,
				initialNetworkHosts: grants.networkHosts,
				toolCallId: id,
				blockedPaths: [
					sandboxState.config.brokerPath ?? PACKAGED_BROKER_PATH,
				],
				approve: (request, approvalSignal) =>
					promptForNativeApproval(
						request,
						{
							toolName: "bash",
							toolCallId: id,
							signal: approvalSignal,
						},
						ctx,
					),
			});
			const result = await createBashTool(localCwd, { operations }).execute(
				id,
				params,
				signal,
				onUpdate,
			);
			return {
				...result,
				content: result.content.map((item) =>
					item.type === "text"
						? { ...item, text: modelVisibleApprovedRetryOutput(item.text) }
						: item,
				),
			};
		},
	});

	pi.on("tool_call", async (event, ctx) => {
		if (event.toolName === "bash" || event.toolName === "request_network_permission") return;
		if (!["read", "write", "edit", "grep", "find", "ls"].includes(event.toolName)) return;
		if (event.toolName === "grep" || event.toolName === "find") {
			return {
				block: true,
				reason:
					`Use ${event.toolName === "grep" ? "rg" : "fd"} through bash. ` +
					"The built-in recursive tool cannot inherit the OS file policy.",
			};
		}
		const lexicalPath = toolLexicalPath(event, ctx.cwd);
		if (!lexicalPath) return { block: true, reason: "File path is missing" };
		const path = canonicalize(lexicalPath);
		const access = event.toolName === "write" || event.toolName === "edit" ? "write" : "read";
		if (
			isProtectedPath(lexicalPath) ||
			(access === "write" && isProtectedWritePath(lexicalPath)) ||
			isDeniedByConfig(path, access, activeConfig(sandboxState), ctx.cwd)
		) {
			return { block: true, reason: `Protected or denied ${access} path: ${path}` };
		}
		const gitRoot = access === "write" ? gitControlRoot(lexicalPath, ctx.cwd) : undefined;
		const piRoot = access === "write" ? projectControlRoot(lexicalPath, ctx.cwd) : undefined;
		const controlRoot = gitRoot ?? piRoot;
		if (controlRoot && isControlRootSymlink(controlRoot)) {
			return {
				block: true,
				reason: `Writes to a symlinked control folder cannot be granted: ${controlRoot}`,
			};
		}
		const allowed = controlRoot
			? persistentPermissions.some(
					(permission) =>
						permission.kind === access &&
						permission.directory &&
						lexicalControlKey(permission.path) === lexicalControlKey(controlRoot),
				)
			: persistentPermissions.some(
					(permission) =>
						permission.kind === access && permissionCoversPath(permission, path),
				) ||
				(access === "read"
					? isBaseReadAllowed(path, activeConfig(sandboxState), ctx.cwd)
					: isBaseWriteAllowed(path, activeConfig(sandboxState), ctx.cwd));
		if (!allowed) {
			const permissionPath = controlRoot ?? path;
			const permission: IoPermission = {
				kind: access,
				path: permissionPath,
				directory:
					controlRoot !== undefined ||
					event.toolName === "ls" ||
					(existsSync(permissionPath) && statSync(permissionPath).isDirectory()),
			};
			const decision = await promptForToolPermission(permission, event, ctx);
			if (!decision.allowed) return { block: true, reason: decision.reason };
		}
		if ("path" in event.input && typeof event.input.path === "string") {
			event.input.path = path;
		}
	});

	pi.on("user_bash", () => {
		if (sandboxState.kind === "disabled") return;
		if (sandboxState.kind === "ready") {
			if (!brokerClient) {
				return { operations: unavailableBashOps("Native sandbox broker is not ready") };
			}
			const filePermissions = persistentPermissions.filter(isSafeSavedFilePermission);
			return {
				operations: createNativeSandboxOps(
					brokerClient,
					sandboxState.config,
					filePermissions,
					[],
					`user-bash-${++userBashCounter}-${randomUUID()}`,
				),
			};
		}
		return {
			operations: unavailableBashOps(
				sandboxState.kind === "failed"
					? sandboxState.reason
					: "Sandbox is still initializing; command blocked",
			),
		};
	});

	pi.on("session_start", async (_event, ctx) => {
		const generation = ++sessionGeneration;
		persistentPermissions = loadWorkspacePermissions(permissionFile, ctx.cwd);
		if (pi.getFlag("no-sandbox") as boolean) {
			sandboxState = { kind: "disabled", reason: "disabled via --no-sandbox" };
			ctx.ui.notify("Sandbox disabled via --no-sandbox", "warning");
			return;
		}
		try {
			const config = loadConfig(ctx.cwd, ctx.isProjectTrusted());
			if (!config.enabled) {
				sandboxState = { kind: "disabled", reason: "disabled via global config" };
				ctx.ui.notify("Sandbox disabled via global config", "warning");
				return;
			}
			sandboxState = { kind: "initializing" };
			ensureDevelopmentCacheDirectories(config.developmentCache);
			if (process.platform !== "darwin" && process.platform !== "linux") {
				throw new Error("the native sandbox supports macOS and Linux only");
			}
			const brokerPath = config.brokerPath ?? PACKAGED_BROKER_PATH;
			const client = await SandboxBrokerClient.start(brokerPath);
			if (generation !== sessionGeneration) {
				await client.shutdown();
				return;
			}
			brokerClient = client;
			backgroundJobs = new NativeBackgroundJobs(brokerPath);
			if (generation !== sessionGeneration) return;
			sandboxState = { kind: "ready", config };
			const backendLabel = `native ${process.platform === "linux" ? "Bubblewrap" : "Seatbelt"}`;
			ctx.ui.setStatus(
				"sandbox",
				ctx.ui.theme.fg("accent", `🔒 ${backendLabel}`),
			);
			ctx.ui.notify(`${backendLabel} sandbox ready`, "info");
		} catch (error) {
			if (generation !== sessionGeneration) return;
			const reason = `Sandbox unavailable; commands are blocked: ${
				error instanceof Error ? error.message : error
			}`;
			sandboxState = { kind: "failed", reason };
			ctx.ui.notify(reason, "error");
		}
	});

	pi.on("session_shutdown", async () => {
		sessionGeneration += 1;
		const client = brokerClient;
		brokerClient = undefined;
		const jobs = backgroundJobs;
		backgroundJobs = undefined;
		if (jobs) await jobs.shutdown();
		if (client) await client.shutdown();
		persistentPermissions = [];
		userBashCounter = 0;
		sandboxState = { kind: "initializing" };
	});

	pi.registerCommand("sandbox", {
		description: "Show OS sandbox rights",
		handler: async (_args, ctx) => {
			if (sandboxState.kind !== "ready") {
				ctx.ui.notify(
					sandboxState.kind === "disabled"
						? `Sandbox is ${sandboxState.reason}`
						: sandboxState.kind === "failed"
							? sandboxState.reason
							: "Sandbox is initializing",
					sandboxState.kind === "failed" ? "error" : "info",
				);
				return;
			}
			const config = sandboxState.config;
			const allowedDomains = config.network?.allowedDomains ?? [];
			const savedHosts = persistentPermissions
				.filter((permission): permission is NetworkPermission => permission.kind === "network_host")
				.map((permission) => permission.host);
			const networkHosts = [...new Set([...allowedDomains, ...savedHosts])].sort();
			const unixSockets = config.network?.allowUnixSockets ?? [];
			ctx.ui.notify(
				[
					`OS sandbox (native-preview):`,
					`  Read: ${config.filesystem?.allowRead?.join(", ") || "(minimal only)"}`,
					`  Write: ${config.filesystem?.allowWrite?.join(", ") || "(workspace only)"}`,
					`  Shell env: ${config.shellEnvironment?.inherit ?? "core"}, secret-name filter ${
						config.shellEnvironment?.ignoreDefaultExcludes ? "off" : "on"
					}`,
					`  Development cache: ${developmentCacheRoot(config.developmentCache)}`,
					`  Network hosts: ${
						config.network?.enabled === false
								? "off"
								: networkHosts.length > 0
									? networkHosts.join(", ")
									: "blocked until an exact host or IP is approved"
					}`,
					`  Unix sockets: ${unixSockets.join(", ") || "(none)"}`,
					"  Background jobs: session-scoped native broker jobs",
					"  Denial hints: best effort",
					`  Saved workspace rights: ${persistentPermissions.map(permissionLabel).join(", ") || "(none)"}`,
				].join("\n"),
				"info",
			);
		},
	});
}

function activeConfig(state: SandboxState): NativeSandboxConfig {
	return state.kind === "ready" ? state.config : DEFAULT_CONFIG;
}

function lexicalControlKey(path: string): string {
	return resolve(canonicalize(dirname(path)), basename(path));
}

function toolLexicalPath(event: ToolCallEvent, cwd: string): string | undefined {
	if (!("path" in event.input) || event.input.path === undefined) {
		return event.toolName === "ls" ? resolve(cwd) : undefined;
	}
	if (typeof event.input.path !== "string") return undefined;
	return resolveLexicalPermissionPath(event.input.path, cwd);
}
