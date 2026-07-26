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
	grantsToRuntime,
	type IoPermission,
	isProtectedPath,
	isProtectedWritePath,
	loadWorkspacePermissions,
	mcpPermissionFromInput,
	permissionCoversPath,
	permissionLabel,
	resolvePermissionPath,
	saveWorkspacePermission,
} from "./io-permissions.ts";
import {
	isBaseReadAllowed,
	isBaseWriteAllowed,
	isDeniedByConfig,
} from "./io-policy.ts";
import { parseFilesystemDenials } from "./sandbox-denials.ts";

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

interface ApprovalDecision {
	allowed: boolean;
	persistent: boolean;
	reason?: string;
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

	const runtimeGrants = (): CodexSandboxGrants => {
		const grants = grantsToRuntime(persistentPermissions);
		return {
			read: grants.read,
			write: grants.write,
			web: grants.web,
			localNetwork: grants.localNetwork,
		};
	};

	const promptForToolPermission = async (
		permission: IoPermission,
		tool: { toolName: string; toolCallId: string },
		ctx: ExtensionContext,
	): Promise<ApprovalDecision> => {
		const label = permissionLabel(permission);
		if (!ctx.hasUI) {
			return { allowed: false, persistent: false, reason: `${label} needs user approval` };
		}

		pi.events.emit("approval:requested", {
			kind: "io-permission",
			title: "Tool requests an IO right",
			summary: `${tool.toolName} requests ${label}`,
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
			`Allow ${tool.toolName} to access ${label}?`,
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
				web: initialGrants.web,
				localNetwork: initialGrants.localNetwork,
			};
			let lastResult: { exitCode: number | null } = { exitCode: 1 };

			for (let attempt = 0; attempt < 4; attempt++) {
				const chunks: Buffer[] = [];
				let displayPending = "";
				let suppressDenialLog = false;
				lastResult = await createCodexSandboxOps(config, grants).exec(command, cwd, {
					...options,
					onData(data) {
						chunks.push(data);
						displayPending += data.toString("utf8");
						let newline = displayPending.indexOf("\n");
						while (newline >= 0) {
							const line = displayPending.slice(0, newline + 1);
							displayPending = displayPending.slice(newline + 1);
							if (line.trim() === "=== Sandbox denials ===") suppressDenialLog = true;
							if (!suppressDenialLog) options.onData(Buffer.from(line));
							newline = displayPending.indexOf("\n");
						}
					},
				});
				if (!suppressDenialLog && displayPending) {
					options.onData(Buffer.from(displayPending));
				}
				if (lastResult.exitCode === 0 || options.signal?.aborted) return lastResult;

				const denials = parseFilesystemDenials(Buffer.concat(chunks).toString("utf8"));
				if (denials.length === 0) return lastResult;
				const permissions: FilePermission[] = [];
				for (const denial of denials) {
					const path = resolvePermissionPath(denial.path, cwd);
					if (
						isProtectedPath(path) ||
						(denial.access === "write" && isProtectedWritePath(path)) ||
						isDeniedByConfig(path, denial.access, config, cwd)
					) {
						return lastResult;
					}
					permissions.push({
						kind: denial.access,
						path,
						directory: existsSync(path) && statSync(path).isDirectory(),
					});
				}

				for (const permission of permissions) {
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
			const grants = runtimeGrants();
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
		if (event.toolName === "bash") return;
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
			const permission: IoPermission = {
				kind: access,
				path,
				directory:
					event.toolName === "ls" || (existsSync(path) && statSync(path).isDirectory()),
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
			const savedWeb = persistentPermissions.some(
				(permission) => permission.kind === "web",
			);
			const allowedDomains = config.network?.allowedDomains ?? [];
			ctx.ui.notify(
				[
					"Codex IO sandbox:",
					`  Read: ${config.filesystem?.allowRead?.join(", ") || "(minimal only)"}`,
					`  Write: ${config.filesystem?.allowWrite?.join(", ") || "(workspace only)"}`,
					`  Shell env: ${config.shellEnvironment?.inherit ?? "core"}, secret-name filter ${
						config.shellEnvironment?.ignoreDefaultExcludes ? "off" : "on"
					}`,
					`  Public web: ${
						config.network?.enabled === false
							? "off"
							: savedWeb
								? "all public hosts (saved)"
								: allowedDomains.length > 0
									? allowedDomains.join(", ")
									: "blocked until approved"
					}`,
					`  Local/private/link-local network: ${
						(config.network?.allowLocalNetwork ?? false) ||
						persistentPermissions.some(
							(permission) => permission.kind === "local_network",
						)
							? "all allowed"
							: "blocked until approved"
					}`,
					`  Unix sockets: ${config.network?.allowUnixSockets?.join(", ") || "(none)"}`,
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
