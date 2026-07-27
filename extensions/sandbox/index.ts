/**
 * Pi sandbox and explicit IO permission broker.
 *
 * Commands may use any interpreter or child process. Codex applies the same
 * filesystem and network profile to the whole process tree. The model may ask
 * for a concrete IO right, but it may not ask to bypass the sandbox.
 */

import { spawn } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import { resolve } from "node:path";
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
import {
	isBackgroundJobSocket,
	isValidBackgroundJobName,
	runBackgroundJobHelper,
	sandboxedJobCommand,
} from "./background-jobs.ts";
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
	isProtectedPath,
	isProtectedWritePath,
	loadWorkspacePermissions,
	mcpPermissionFromInput,
	permissionCoversPath,
	permissionLabel,
	projectControlRoot,
	normalizePermission,
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

type FilePermission = Extract<IoPermission, { kind: "read" | "write" }>;
type NetworkPermission = Extract<IoPermission, { kind: "network_host" }>;

const NetworkPermissionParams = Type.Object({
	host: Type.String({
		description: "One exact hostname or IP, with no scheme, port, path, or wildcard",
	}),
	reason: Type.String({ description: "Why this host is needed" }),
});

const BackgroundJobParams = Type.Union([
	Type.Object({
		action: Type.Literal("start"),
		name: Type.String({ description: "Unique job name starting with pi-" }),
		command: Type.String({ description: "Shell command to run in the background" }),
		cwd: Type.Optional(Type.String({ description: "Working directory inside this workspace" })),
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
	let oneShotNetworkPermissions: NetworkPermission[] = [];

	const runtimeGrants = (consumeOneShot = false): CodexSandboxGrants => {
		const permissions = [...persistentPermissions, ...oneShotNetworkPermissions];
		if (consumeOneShot) oneShotNetworkPermissions = [];
		const grants = grantsToRuntime(permissions);
		return {
			read: grants.read,
			write: grants.write,
			networkHosts: grants.networkHosts,
		};
	};

	const promptForToolPermission = async (
		permission: IoPermission,
		tool: { toolName: string; toolCallId: string; reason?: string },
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
		const allowOnce = tool.toolName === "bash" ? "Allow once and retry" : "Allow once";
		const allowAlways =
			tool.toolName === "bash"
				? "Always allow in this workspace and retry"
				: "Always allow in this workspace";
		const selection = await ctx.ui.select(
			`Allow ${tool.toolName} to access ${label}?${
				tool.reason ? `\n\nReason: ${tool.reason}` : ""
			}`,
			[allowOnce, allowAlways, "No", "No, with comment"],
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
				lastResult = await createCodexSandboxOps(config, grants).exec(command, cwd, {
					...options,
					onData(data) {
						chunks.push(data);
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
					const path = resolvePermissionPath(failurePath, cwd);
					const gitRoot = gitControlRoot(path);
					const piRoot = projectControlRoot(path, cwd);
					const permissionPath = gitRoot ?? piRoot ?? path;
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
					const decision = await promptForToolPermission(permission, tool, ctx);
					if (!decision.allowed) return lastResult;
					const paths = permission.kind === "read" ? grants.read! : grants.write!;
					if (!paths.includes(permission.path)) paths.push(permission.path);
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
			"Ask the user to allow one exact hostname or IP for the next bash or background job start, or for this workspace. Do not include a scheme, port, path, or wildcard.",
		promptSnippet:
			"Use request_network_permission before bash or a background job needs a network host that the sandbox has not allowed.",
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
				...grantsToRuntime([
					...persistentPermissions,
					...oneShotNetworkPermissions,
				]).networkHosts,
			].some((rule) => networkRuleMatches(rule, permission.host));
			if (existing) {
				return {
					content: [{ type: "text", text: `${permission.host} is already allowed.` }],
					details: { granted: true, existing: true },
				};
			}
			const decision = await promptForToolPermission(
				permission,
				{ toolName: "request_network_permission", toolCallId, reason: params.reason },
				ctx,
			);
			if (decision.allowed && !decision.persistent) {
				oneShotNetworkPermissions.push(permission);
			}
			return {
				content: [
					{
						type: "text",
						text: decision.allowed
							? `${permission.host} is allowed ${
									decision.persistent
										? "for this workspace"
										: "for the next bash or background job start"
								}.`
							: decision.reason ?? "Network host denied.",
					},
				],
				details: {
					granted: decision.allowed,
					scope: decision.allowed
						? decision.persistent
							? "workspace"
							: "once"
						: "none",
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
			"Start, list, inspect, interact with, or stop a long-running command. Started commands run in a fresh Codex sandbox. Job names must start with pi-.",
		promptSnippet:
			"Use background_job for long-running servers, watchers, builds, and tests instead of calling tmux through bash.",
		parameters: BackgroundJobParams,
		executionMode: "sequential",
		async execute(_toolCallId, params, signal, _onUpdate, ctx) {
			const packagedHelperPath = fileURLToPath(new URL("./background-job.sh", import.meta.url));
			const sourceHelperPath = fileURLToPath(
				new URL("../../skills/background-jobs/scripts/job.sh", import.meta.url),
			);
			const skillHelperPath = resolve(
				getAgentDir(),
				"skills",
				"background-jobs",
				"scripts",
				"job.sh",
			);
			const helperPath = [packagedHelperPath, sourceHelperPath, skillHelperPath].find(existsSync);
			if (!helperPath) {
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
				const grants = runtimeGrants(true);
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
		label: "bash (Codex sandbox)",
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
			const grants = runtimeGrants(true);
			return createBashTool(localCwd, {
				operations: createApprovingSandboxOps(
					sandboxState.config,
					grants,
					{ toolName: "bash", toolCallId: id },
					ctx,
				),
			}).execute(id, params, signal, onUpdate);
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
		const path = toolPath(event, ctx.cwd);
		if (!path) return { block: true, reason: "File path is missing" };
		const access = event.toolName === "write" || event.toolName === "edit" ? "write" : "read";
		if (
			isProtectedPath(path) ||
			(access === "write" && isProtectedWritePath(path)) ||
			isDeniedByConfig(path, access, activeConfig(sandboxState), ctx.cwd)
		) {
			return { block: true, reason: `Protected or denied ${access} path: ${path}` };
		}
		const allowed =
			persistentPermissions.some(
				(permission) =>
					permission.kind === access && permissionCoversPath(permission, path),
			) ||
			(access === "read"
				? isBaseReadAllowed(path, activeConfig(sandboxState), ctx.cwd)
				: isBaseWriteAllowed(path, activeConfig(sandboxState), ctx.cwd));
		if (!allowed) {
			const gitRoot = access === "write" ? gitControlRoot(path) : undefined;
			const piRoot = access === "write" ? projectControlRoot(path, ctx.cwd) : undefined;
			const permissionPath = gitRoot ?? piRoot ?? path;
			const permission: IoPermission = {
				kind: access,
				path: permissionPath,
				directory:
					gitRoot !== undefined ||
					piRoot !== undefined ||
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
			await checkCodex(config.codexCommand ?? DEFAULT_CONFIG.codexCommand);
			sandboxState = { kind: "ready", config };
			ctx.ui.setStatus(
				"sandbox",
				ctx.ui.theme.fg(
					"accent",
					`🔒 Codex IO sandbox: ${config.permissionProfile ?? DEFAULT_CONFIG.permissionProfile}`,
				),
			);
			ctx.ui.notify("Codex IO sandbox ready", "info");
		} catch (error) {
			const reason = `Codex sandbox unavailable; commands are blocked: ${
				error instanceof Error ? error.message : error
			}`;
			sandboxState = { kind: "failed", reason };
			ctx.ui.notify(reason, "error");
		}
	});

	pi.on("session_shutdown", () => {
		persistentPermissions = [];
		oneShotNetworkPermissions = [];
		sandboxState = { kind: "initializing" };
	});

	pi.registerCommand("sandbox", {
		description: "Show Codex sandbox rights",
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
			const unixSockets = (config.network?.allowUnixSockets ?? []).filter(
				(socket) => !isBackgroundJobSocket(socket, canonicalize),
			);
			ctx.ui.notify(
				[
					"Codex IO sandbox:",
					`  Read: ${config.filesystem?.allowRead?.join(", ") || "(minimal only)"}`,
					`  Write: ${config.filesystem?.allowWrite?.join(", ") || "(workspace only)"}`,
					`  Shell env: ${config.shellEnvironment?.inherit ?? "core"}, secret-name filter ${
						config.shellEnvironment?.ignoreDefaultExcludes ? "off" : "on"
					}`,
					`  Network hosts: ${
						config.network?.enabled === false
							? "off"
							: networkHosts.length > 0
								? networkHosts.join(", ")
								: "blocked until an exact host or IP is approved"
					}`,
					`  Unix sockets: ${unixSockets.join(", ") || "(none)"}`,
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

function toolPath(event: ToolCallEvent, cwd: string): string | undefined {
	if (!("path" in event.input) || event.input.path === undefined) {
		return event.toolName === "ls" ? resolvePermissionPath(cwd, cwd) : undefined;
	}
	if (typeof event.input.path !== "string") return undefined;
	return resolvePermissionPath(event.input.path, cwd);
}
