/**
 * Pi sandbox and explicit IO permission broker.
 *
 * Commands may use any interpreter or child process. Codex applies the same
 * filesystem and network profile to the whole process tree. The model may ask
 * for a concrete IO right, but it may not ask to bypass the sandbox.
 */

import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { existsSync, readFileSync, statSync } from "node:fs";
import { basename, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
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
import { buildBrokerExecRequest } from "./broker-policy.ts";
import {
	isBackgroundJobSocket,
	isValidBackgroundJobName,
	runBackgroundJobHelper,
	sandboxedJobCommand,
} from "./background-jobs.ts";
import {
	checkDeclaredFilesystemPermissions,
	type FilePermission,
} from "./declared-permissions.ts";
import {
	DEFAULT_CONFIG,
	applyProjectRestrictions,
	buildCodexSandboxArgs,
	buildShellEnvironment,
	type CodexSandboxConfig,
	type CodexSandboxGrants,
	mergeGlobalConfig,
	normalizeConfig,
} from "./codex-command.ts";
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
	mcpPermissionFromInput,
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
import { parseFilesystemFailurePaths } from "./sandbox-failures.ts";

function readConfig(path: string): CodexSandboxConfig | undefined {
	if (!existsSync(path)) return undefined;
	const parsed: unknown = JSON.parse(readFileSync(path, "utf8"));
	return normalizeConfig(parsed);
}

function loadConfig(cwd: string, projectTrusted: boolean): CodexSandboxConfig {
	const globalPath = resolve(getAgentDir(), "extensions", "sandbox.json");
	const projectPath = resolve(cwd, CONFIG_DIR_NAME, "sandbox.json");
	const global = readConfig(globalPath) ?? {};
	const base = mergeGlobalConfig(DEFAULT_CONFIG, global);
	if (!projectTrusted) return base;
	return applyProjectRestrictions(base, readConfig(projectPath) ?? {});
}

function checkCodex(command: string): Promise<void> {
	return new Promise((resolveCheck, reject) => {
		const child = spawn(command, ["sandbox", "--help"], { stdio: "ignore" });
		child.once("error", reject);
		child.once("close", (code) => {
			if (code === 0) resolveCheck();
			else reject(
				new Error(`${command} sandbox --help exited with status ${code ?? "unknown"}`),
			);
		});
	});
}

const PACKAGED_BROKER_PATH = "@PI_SANDBOX_BROKER@/bin/pi-sandbox-broker";
const NATIVE_PREVIEW_RELEASED = false;
const MAX_FAILURE_OUTPUT_BYTES = 1024 * 1024;

function createCodexSandboxOps(
	config: CodexSandboxConfig,
	grants: CodexSandboxGrants,
): BashOperations {
	return {
		async exec(command, cwd, { onData, signal, timeout }) {
			if (!existsSync(cwd)) throw new Error(`Working directory does not exist: ${cwd}`);
			const args = buildCodexSandboxArgs(cwd, config, command, grants);
			const codexCommand = config.codexCommand ?? DEFAULT_CONFIG.codexCommand;

			return new Promise((resolveExec, reject) => {
				const child = spawn(codexCommand, args, {
					cwd,
					detached: true,
					env: {
						...buildShellEnvironment(config),
						IN_SANDBOX: "1",
						PI_SANDBOX: "codex",
					},
					stdio: ["ignore", "pipe", "pipe"],
				});
				let timedOut = false;
				let timeoutHandle: ReturnType<typeof setTimeout> | undefined;
				const kill = () => {
					if (!child.pid) return;
					try {
						process.kill(-child.pid, "SIGKILL");
					} catch {
						child.kill("SIGKILL");
					}
				};
				if (timeout !== undefined && timeout > 0) {
					timeoutHandle = setTimeout(() => {
						timedOut = true;
						kill();
					}, timeout * 1000);
				}
				child.stdout?.on("data", onData);
				child.stderr?.on("data", onData);
				child.once("error", (error) => {
					if (timeoutHandle) clearTimeout(timeoutHandle);
					signal?.removeEventListener("abort", kill);
					reject(error);
				});
				signal?.addEventListener("abort", kill, { once: true });
				child.once("close", (code) => {
					if (timeoutHandle) clearTimeout(timeoutHandle);
					signal?.removeEventListener("abort", kill);
					if (signal?.aborted) reject(new Error("aborted"));
					else if (timedOut) reject(new Error(`timeout:${timeout}`));
					else resolveExec({ exitCode: code ?? 1 });
				});
			});
		},
	};
}

function createNativeSandboxOps(
	client: SandboxBrokerClient,
	config: CodexSandboxConfig,
	permissions: readonly FilePermission[],
	networkHosts: readonly string[],
	commandId: string,
): BashOperations {
	return {
		async exec(command, cwd, { onData, signal, timeout }) {
			const request = buildBrokerExecRequest(
				commandId,
				command,
				cwd,
				timeout,
				config,
				permissions,
				networkHosts,
			);
			return client.exec(request, onData, signal);
		},
	};
}

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
	| { kind: "ready"; config: CodexSandboxConfig }
	| { kind: "failed"; reason: string };

type NetworkPermission = Extract<IoPermission, { kind: "network_host" }>;
type DeclaredFilesystemPermission = {
	kind: "read" | "write";
	path: string;
	targetType?: "file" | "folder";
	reason: string;
};
type DeclaredNetworkPermission = {
	kind: "network_host";
	host: string;
	reason: string;
};

const NetworkPermissionParams = Type.Object({
	host: Type.String({
		description: "One exact hostname or IP, with no scheme, port, path, or wildcard",
	}),
	reason: Type.String({ description: "Why this host is needed" }),
});

const DeclaredFilesystemPermissionParams = Type.Object({
	kind: Type.Union([Type.Literal("read"), Type.Literal("write")]),
	path: Type.String({ description: "Exact file or folder path" }),
	targetType: Type.Optional(
		Type.Union([Type.Literal("file"), Type.Literal("folder")]),
	),
	reason: Type.String({ description: "Why this command needs the right" }),
});

const DeclaredNetworkPermissionParams = Type.Object({
	kind: Type.Literal("network_host"),
	host: Type.String({
		description: "One exact hostname or IP, with no scheme, port, path, or wildcard",
	}),
	reason: Type.String({ description: "Why this command needs the host" }),
});

const DeclaredPermissionsParams = Type.Optional(
	Type.Array(
		Type.Union([
			DeclaredFilesystemPermissionParams,
			DeclaredNetworkPermissionParams,
		]),
		{ maxItems: 16 },
	),
);

const BashParams = Type.Object({
	command: Type.String({ description: "Bash command to execute" }),
	timeout: Type.Optional(
		Type.Number({ description: "Timeout in seconds (optional, no default timeout)" }),
	),
	permissions: DeclaredPermissionsParams,
});

const BackgroundJobParams = Type.Union([
	Type.Object({
		action: Type.Literal("start"),
		name: Type.String({ description: "Unique job name starting with pi-" }),
		command: Type.String({ description: "Shell command to run in the background" }),
		cwd: Type.Optional(Type.String({ description: "Working directory inside this workspace" })),
		permissions: DeclaredPermissionsParams,
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

function fileDeclarations(
	permissions: readonly (DeclaredFilesystemPermission | DeclaredNetworkPermission)[] | undefined,
): DeclaredFilesystemPermission[] {
	return (permissions ?? []).filter(
		(permission): permission is DeclaredFilesystemPermission =>
			permission.kind === "read" || permission.kind === "write",
	);
}

function networkDeclarations(
	permissions: readonly (DeclaredFilesystemPermission | DeclaredNetworkPermission)[] | undefined,
): DeclaredNetworkPermission[] {
	return (permissions ?? []).filter(
		(permission): permission is DeclaredNetworkPermission =>
			permission.kind === "network_host",
	);
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
	let userBashCounter = 0;
	let sessionGeneration = 0;

	pi.on("session_start", () => {
		pi.setActiveTools(
			pi.getActiveTools().filter((name) => name !== "grep" && name !== "find"),
		);
	});

	const runtimeGrants = (): CodexSandboxGrants => {
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
		},
		ctx: ExtensionContext,
	): Promise<ApprovalDecision> => {
		const label = permissionLabel(permission);
		if (!ctx.hasUI) {
			return { allowed: false, persistent: false, reason: `${label} needs user approval` };
		}

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
		const choices =
			tool.allowOnce === false
				? [allowAlways, "No", "No, with comment"]
				: [allowOnce, allowAlways, "No", "No, with comment"];
		const selection = await ctx.ui.select(
			`Allow ${tool.toolName} to access ${label}?${
				tool.reason ? `\n\nReason: ${tool.reason}` : ""
			}`,
			choices,
		);
		let comment: string | undefined;
		if (selection === "No, with comment") {
			comment = await ctx.ui.input("Tell the agent what to do instead", "Short note");
		}
		const allow = selection === allowOnce || selection === allowAlways;
		if (selection === allowAlways) {
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
				persistent: selection === allowAlways,
			};
		}
		return {
			allowed: false,
			persistent: false,
			reason: comment ? `Permission denied. User comment: ${comment}` : "Permission denied by user",
		};
	};

	const approveDeclaredFilesystemPermissions = async (
		declarations: readonly DeclaredFilesystemPermission[] | undefined,
		tool: { toolName: string; toolCallId: string },
		ctx: ExtensionContext,
		config: CodexSandboxConfig,
	): Promise<FilePermission[]> => {
		const checked = checkDeclaredFilesystemPermissions(
			declarations,
			ctx.cwd,
			config,
			persistentPermissions,
		);
		for (const entry of checked) {
			if (entry.alreadyAllowed) continue;
			const decision = await promptForToolPermission(
				entry.permission,
				{ ...tool, reason: entry.reason, retry: false },
				ctx,
			);
			if (!decision.allowed) {
				throw new Error(decision.reason ?? "Permission denied by user");
			}
		}
		return checked.map((entry) => entry.permission);
	};

	const approveDeclaredNetworkPermissions = async (
		declarations: readonly DeclaredNetworkPermission[] | undefined,
		tool: { toolName: string; toolCallId: string },
		ctx: ExtensionContext,
		config: CodexSandboxConfig,
	): Promise<NetworkPermission[]> => {
		if ((declarations?.length ?? 0) > 16) {
			throw new Error("A command may declare at most 16 network hosts");
		}
		if ((declarations?.length ?? 0) > 0 && config.backend === "native-preview") {
			throw new Error("Network access is not available in the native sandbox preview");
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

	const createApprovingSandboxOps = (
		config: CodexSandboxConfig,
		initialGrants: CodexSandboxGrants,
		tool: { toolName: string; toolCallId: string },
		ctx: ExtensionContext,
	): BashOperations => ({
		async exec(command, cwd, options) {
			const grants: CodexSandboxGrants = {
				read: [...(initialGrants.read ?? [])],
				write: [...(initialGrants.write ?? [])],
				networkHosts: [...(initialGrants.networkHosts ?? [])],
			};
			let lastResult: { exitCode: number | null } = { exitCode: 1 };

			for (let attempt = 0; attempt < 4; attempt++) {
				const chunks: Buffer[] = [];
				let retainedBytes = 0;
				lastResult = await createCodexSandboxOps(config, grants).exec(command, cwd, {
					...options,
					onData(data) {
						if (retainedBytes < MAX_FAILURE_OUTPUT_BYTES) {
							const remaining = MAX_FAILURE_OUTPUT_BYTES - retainedBytes;
							const retained = data.subarray(0, remaining);
							chunks.push(retained);
							retainedBytes += retained.length;
						}
						options.onData(data);
					},
				});
				if (lastResult.exitCode === 0 || options.signal?.aborted) return lastResult;

				const failurePaths = parseFilesystemFailurePaths(
					Buffer.concat(chunks).toString("utf8"),
				);
				if (failurePaths.length === 0) return lastResult;
				const permissions = new Map<string, FilePermission>();
				for (const failurePath of failurePaths) {
					const lexicalPath = resolveLexicalPermissionPath(failurePath, cwd);
					const path = canonicalize(lexicalPath);
					const gitRoot = gitControlRoot(lexicalPath, cwd);
					const piRoot = projectControlRoot(lexicalPath, cwd);
					const controlRoot = gitRoot ?? piRoot;
					if (controlRoot && isControlRootSymlink(controlRoot)) return lastResult;
					const permissionPath = controlRoot ?? path;
					// The current profile can read the whole filesystem. A path that is
					// readable but not writable therefore identifies a write denial. If
					// policy does not identify one exact access kind, treat it as a
					// regular command failure rather than asking for broad access.
					const readAllowed = isBaseReadAllowed(path, config, cwd);
					const writeAllowed = isBaseWriteAllowed(path, config, cwd);
					if (!readAllowed || writeAllowed) return lastResult;
					const access = "write" as const;
					const alreadyGranted = (grants.write ?? []).some((root) => {
						const grantPath = resolvePermissionPath(root, cwd);
						return permissionCoversPath(
							{
								kind: "write",
								path: grantPath,
								directory: existsSync(grantPath) && statSync(grantPath).isDirectory(),
							},
							path,
						);
					});
					if (alreadyGranted) return lastResult;
					if (
						isProtectedPath(path) ||
						isProtectedWritePath(path) ||
						isDeniedByConfig(path, access, config, cwd)
					) {
						return lastResult;
					}
					permissions.set(permissionPath, {
						kind: access,
						path: permissionPath,
						directory:
							gitRoot !== undefined ||
							piRoot !== undefined ||
							(existsSync(permissionPath) && statSync(permissionPath).isDirectory()),
					});
				}

				for (const permission of permissions.values()) {
					const decision = await promptForToolPermission(
						permission,
						{ ...tool, retry: true },
						ctx,
					);
					if (!decision.allowed) return lastResult;
					const paths = permission.kind === "read" ? grants.read! : grants.write!;
					const runtimePath = canonicalize(permission.path);
					if (!paths.includes(runtimePath)) paths.push(runtimePath);
				}
				options.onData(Buffer.from("\n[Retrying command with approved IO rights]\n"));
			}
			return lastResult;
		},
	});

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
			if (config.backend === "native-preview") {
				return {
					content: [
						{
							type: "text",
							text: "Network access is not available in the native sandbox preview.",
						},
					],
					details: { granted: false, reason: "native-network-unsupported" },
					isError: true,
				};
			}
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
			"Start, list, inspect, interact with, or stop a long-running command. Use only for long-running commands; never run sudo, password prompts, or destructive commands. Jobs run in a fresh Codex sandbox inside the workspace. A start may declare exact read, write, or network_host rights so the user can approve them before launch. Names must start with pi- and use only letters, digits, dots, underscores, or hyphens. After starting, do other work instead of tight polling. Stop only jobs created for the current task and report any left running when the task ends.",
		promptSnippet:
			"Use background_job for long-running servers, watchers, builds, and tests instead of tmux through bash.",
		parameters: BackgroundJobParams,
		executionMode: "sequential",
		async execute(toolCallId, params, signal, _onUpdate, ctx) {
			const helperPath = fileURLToPath(new URL("./background-job.sh", import.meta.url));
			if (!existsSync(helperPath)) {
				return {
					content: [{ type: "text", text: "Background job helper is missing" }],
					isError: true,
				};
			}
			const config = activeConfig(sandboxState);
			const environment = {
				...buildShellEnvironment(config),
				IN_SANDBOX: "1",
				PI_SANDBOX: "codex",
			};
			let args: string[];
			if ("name" in params && !isValidBackgroundJobName(params.name)) {
				return {
					content: [
						{
							type: "text",
							text: "Job names must start with pi- and use only letters, digits, dots, underscores, or hyphens.",
						},
					],
					isError: true,
				};
			}
			if (params.action === "start") {
				if (sandboxState.kind !== "ready") {
					return {
						content: [
							{
								type: "text",
								text: "The sandbox is not ready, so no background job was started.",
							},
						],
						isError: true,
					};
				}
				if (config.backend === "native-preview") {
					return {
						content: [
							{
								type: "text",
								text: "Background jobs are not available in the native sandbox preview.",
							},
						],
						isError: true,
					};
				}
				const cwd = resolvePermissionPath(params.cwd ?? ctx.cwd, ctx.cwd);
				if (!isInside(canonicalize(ctx.cwd), cwd)) {
					return {
						content: [{ type: "text", text: "Background jobs must start inside the current workspace." }],
						isError: true,
					};
				}
				if (!existsSync(cwd) || !statSync(cwd).isDirectory()) {
					return {
						content: [{ type: "text", text: `Background job directory does not exist: ${cwd}` }],
						isError: true,
					};
				}
				const declaredNetworkPermissions = await approveDeclaredNetworkPermissions(
					networkDeclarations(params.permissions),
					{ toolName: "background_job", toolCallId },
					ctx,
					config,
				);
				const declaredPermissions = await approveDeclaredFilesystemPermissions(
					fileDeclarations(params.permissions),
					{ toolName: "background_job", toolCallId },
					ctx,
					config,
				);
				const grants = runtimeGrants();
				const declaredGrants = grantsToRuntime([
					...declaredPermissions,
					...declaredNetworkPermissions,
				]);
				grants.read = [...(grants.read ?? []), ...declaredGrants.read];
				grants.write = [...(grants.write ?? []), ...declaredGrants.write];
				grants.networkHosts = [
					...(grants.networkHosts ?? []),
					...declaredGrants.networkHosts,
				];
				const codexCommand = config.codexCommand ?? DEFAULT_CONFIG.codexCommand;
				const command = sandboxedJobCommand(
					codexCommand,
					buildCodexSandboxArgs(cwd, config, params.command, grants),
					environment,
				);
				args = ["start", params.name, cwd, command];
			} else if (params.action === "list") {
				args = ["list"];
			} else if (params.action === "status" || params.action === "stop") {
				args = [params.action, params.name];
			} else if (params.action === "read") {
				args = ["read", params.name, String(params.lines ?? 200)];
			} else if (params.action === "write" || params.action === "line") {
				args = [params.action, params.name, params.text];
			} else {
				args = ["keys", params.name, ...params.keys];
			}

			const result = await runBackgroundJobHelper(helperPath, args, {
				cwd: ctx.cwd,
				environment,
				signal,
			});
			return {
				content: [
					{
						type: "text",
						text:
							result.output ||
							(result.exitCode === 0 ? "Done" : "Background job request failed"),
					},
				],
				details: { action: params.action, exitCode: result.exitCode },
				isError: result.exitCode !== 0,
			};
		},
	});

	pi.registerTool({
		...localBash,
		label: "bash (OS sandbox)",
		description:
			"Execute a bash command in the OS sandbox. Declare any exact extra read, write, or network_host rights in permissions so the user can approve them before launch.",
		promptSnippet:
			"Use permissions on bash to declare exact extra filesystem or command-only network_host rights before launch.",
		parameters: BashParams,
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
			const declaredPermissions = await approveDeclaredFilesystemPermissions(
				fileDeclarations(params.permissions),
				{ toolName: "bash", toolCallId: id },
				ctx,
				sandboxState.config,
			);
			const grants = runtimeGrants();
			const declaredGrants = grantsToRuntime([
				...declaredPermissions,
				...declaredNetworkPermissions,
			]);
			grants.read = [...(grants.read ?? []), ...declaredGrants.read];
			grants.write = [...(grants.write ?? []), ...declaredGrants.write];
			grants.networkHosts = [
				...(grants.networkHosts ?? []),
				...declaredGrants.networkHosts,
			];
			let operations: BashOperations;
			if (sandboxState.config.backend === "native-preview") {
				if (!brokerClient) throw new Error("Native sandbox broker is not ready");
				const filePermissions = [
					...persistentPermissions.filter(
						(permission): permission is FilePermission =>
							permission.kind === "read" || permission.kind === "write",
					),
					...declaredPermissions,
				];
				operations = createNativeSandboxOps(
					brokerClient,
					sandboxState.config,
					filePermissions,
					[],
					`${id}/attempt-0`,
				);
			} else {
				operations = createApprovingSandboxOps(
					sandboxState.config,
					grants,
					{ toolName: "bash", toolCallId: id },
					ctx,
				);
			}
			return createBashTool(localCwd, { operations }).execute(
				id,
				params,
				signal,
				onUpdate,
			);
		},
	});

	pi.on("tool_call", async (event, ctx) => {
		if (event.toolName === "bash" || event.toolName === "request_network_permission") return;
		if (event.toolName === "mcp_enable") {
			const permission = mcpPermissionFromInput(event.input);
			if (!permission) {
				return {
					block: true,
					reason: "MCP service name is missing; access stays blocked",
				};
			}
			if (
				persistentPermissions.some(
					(entry) =>
						entry.kind === "mcp" && entry.server === permission.server,
				)
			) {
				return;
			}
			if (!ctx.hasUI) {
				return {
					block: true,
					reason: `MCP service ${permission.server} needs user approval`,
				};
			}
			const label = permissionLabel(permission);
			pi.events.emit("approval:requested", {
				kind: "io-permission",
				title: "Agent requests service access",
				summary: label,
				toolName: "mcp_enable",
				toolCallId: event.toolCallId,
				sessionId: ctx.sessionManager.getSessionId(),
				cwd: ctx.cwd,
			});
			const selection = await ctx.ui.select(
				`Allow ${label}?`,
				["Allow once", "Always allow in this workspace", "No", "No, with comment"],
			);
			let comment: string | undefined;
			if (selection === "No, with comment") {
				comment = await ctx.ui.input("Tell the agent what to do instead", "Short note");
			}
			const allow =
				selection === "Allow once" ||
				selection === "Always allow in this workspace";
			if (selection === "Always allow in this workspace") {
				saveWorkspacePermission(permissionFile, ctx.cwd, permission);
				persistentPermissions = loadWorkspacePermissions(permissionFile, ctx.cwd);
			}
			pi.events.emit("approval:resolved", {
				kind: "io-permission",
				toolName: "mcp_enable",
				toolCallId: event.toolCallId,
				decision: allow ? "allowed" : "denied",
			});
			if (allow) return;
			return {
				block: true,
				reason: comment
					? `MCP access denied. User comment: ${comment}`
					: "MCP access denied",
			};
		}
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
			if (sandboxState.config.backend === "native-preview") {
				if (!brokerClient) {
					return { operations: unavailableBashOps("Native sandbox broker is not ready") };
				}
				const filePermissions = persistentPermissions.filter(
					(permission): permission is FilePermission =>
						permission.kind === "read" || permission.kind === "write",
				);
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
			return { operations: createCodexSandboxOps(sandboxState.config, runtimeGrants()) };
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
			if (config.backend === "native-preview") {
				if (!NATIVE_PREVIEW_RELEASED) {
					throw new Error(
						"the native sandbox preview remains blocked until its cleanup release gates pass",
					);
				}
				if (process.platform !== "darwin") {
					throw new Error("the native sandbox preview supports macOS only");
				}
				const client = await SandboxBrokerClient.start(
					config.brokerPath ?? PACKAGED_BROKER_PATH,
				);
				if (generation !== sessionGeneration) {
					await client.shutdown();
					return;
				}
				brokerClient = client;
			} else {
				await checkCodex(config.codexCommand ?? DEFAULT_CONFIG.codexCommand);
			}
			if (generation !== sessionGeneration) return;
			sandboxState = { kind: "ready", config };
			const backendLabel =
				config.backend === "native-preview"
					? "native Seatbelt preview (network blocked)"
					: `Codex IO: ${config.permissionProfile ?? DEFAULT_CONFIG.permissionProfile}`;
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
			const native = config.backend === "native-preview";
			const allowedDomains = config.network?.allowedDomains ?? [];
			const savedHosts = persistentPermissions
				.filter((permission): permission is NetworkPermission => permission.kind === "network_host")
				.map((permission) => permission.host);
			const networkHosts = [...new Set([...allowedDomains, ...savedHosts])].sort();
			const unixSockets = (config.network?.allowUnixSockets ?? []).filter(
				(socket) => !isBackgroundJobSocket(socket, canonicalize),
			);
			ctx.ui.notify(
				[
					`OS sandbox (${config.backend ?? "codex"}):`,
					`  Read: ${config.filesystem?.allowRead?.join(", ") || "(minimal only)"}`,
					`  Write: ${config.filesystem?.allowWrite?.join(", ") || "(workspace only)"}`,
					`  Shell env: ${config.shellEnvironment?.inherit ?? "core"}, secret-name filter ${
						config.shellEnvironment?.ignoreDefaultExcludes ? "off" : "on"
					}`,
					`  Network hosts: ${
						native
							? "blocked by native protocol v1"
							: config.network?.enabled === false
								? "off"
								: networkHosts.length > 0
									? networkHosts.join(", ")
									: "blocked until an exact host or IP is approved"
					}`,
					`  Unix sockets: ${native ? "blocked by native protocol v1" : unixSockets.join(", ") || "(none)"}`,
					...(native ? ["  Background jobs: unavailable", "  Denial hints: unavailable"] : []),
					`  Saved workspace rights: ${persistentPermissions.map(permissionLabel).join(", ") || "(none)"}`,
				].join("\n"),
				"info",
			);
		},
	});
}

function activeConfig(state: SandboxState): CodexSandboxConfig {
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
