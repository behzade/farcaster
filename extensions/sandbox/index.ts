/**
 * Pi sandbox and explicit IO permission broker.
 *
 * Commands may use any interpreter or child process. Codex applies the same
 * filesystem and network profile to the whole process tree. The model may ask
 * for a concrete IO right, but it may not ask to bypass the sandbox.
 */

import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { ExtensionAPI, ToolCallEvent } from "@earendil-works/pi-coding-agent";
import {
	type BashOperations,
	CONFIG_DIR_NAME,
	createBashTool,
	getAgentDir,
} from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
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
	isDefaultWritePath,
	isProtectedPath,
	isProtectedWritePath,
	loadWorkspacePermissions,
	mcpPermissionFromInput,
	normalizePermission,
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

const PermissionParams = Type.Union([
	Type.Object({
		kind: Type.Union([Type.Literal("read"), Type.Literal("write")]),
		path: Type.String({ description: "Exact file or folder path" }),
		targetType: Type.Optional(
			Type.Union([Type.Literal("file"), Type.Literal("folder")]),
		),
		reason: Type.String({ description: "Why this IO right is needed" }),
	}),
	Type.Object({
		kind: Type.Literal("web"),
		reason: Type.String({ description: "Why public web access is needed" }),
	}),
	Type.Object({
		kind: Type.Literal("local_port"),
		port: Type.Integer({ minimum: 1, maximum: 65535 }),
		reason: Type.String({ description: "Why this local port is needed" }),
	}),
]);

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
	let oneShotPermissions: IoPermission[] = [];

	const runtimeGrants = (includeOneShot: boolean): CodexSandboxGrants => {
		const permissions = includeOneShot
			? [...persistentPermissions, ...oneShotPermissions]
			: persistentPermissions;
		const grants = grantsToRuntime(permissions);
		return {
			read: grants.read,
			write: grants.write,
			web: grants.web,
			localNetwork: grants.localNetwork,
		};
	};

	pi.registerTool({
		name: "request_io_permission",
		label: "Request IO permission",
		description:
			"Ask the user for one IO right. Request a file or folder read/write right, public web access, or local network access. A local_port request opens every localhost port and may reach private or link-local targets because Codex cannot limit it to one destination port. Do not request permission for a command, executable, interpreter, or script. Most system reads, workspace writes, and temp writes are already allowed.",
		promptSnippet:
			"Use request_io_permission only for an IO right outside the current sandbox.",
		parameters: PermissionParams,
		executionMode: "sequential",
		async execute(toolCallId, params, _signal, _onUpdate, ctx) {
			if (sandboxState.kind !== "ready") {
				return {
					content: [{ type: "text", text: "The sandbox is not ready, so no right was granted." }],
					details: { granted: false, reason: "sandbox-not-ready" },
					isError: true,
				};
			}
			let permission: IoPermission;
			try {
				permission = normalizePermission(params, ctx.cwd);
			} catch (error) {
				return {
					content: [
						{ type: "text", text: error instanceof Error ? error.message : String(error) },
					],
					details: { granted: false, reason: "invalid-request" },
					isError: true,
				};
			}
			if (
				permission.kind === "write" &&
				isDefaultWritePath(permission.path, ctx.cwd) &&
				!isDeniedByConfig(permission.path, "write", sandboxState.config, ctx.cwd)
			) {
				return {
					content: [{ type: "text", text: "That path is already writable." }],
					details: { granted: true, existing: true },
				};
			}
			if (
				permission.kind === "read" &&
				isBaseReadAllowed(permission.path, sandboxState.config, ctx.cwd) &&
				!isDeniedByConfig(permission.path, "read", sandboxState.config, ctx.cwd)
			) {
				return {
					content: [{ type: "text", text: "That path is already readable." }],
					details: { granted: true, existing: true },
				};
			}
			if (permission.kind === "web" && sandboxState.config.network?.enabled === false) {
				return {
					content: [{ type: "text", text: "Web access is disabled by the sandbox policy." }],
					details: { granted: false, reason: "web-disabled" },
					isError: true,
				};
			}
			if (
				permission.kind === "local_network" &&
				sandboxState.config.network?.enabled === false
			) {
				return {
					content: [
						{
							type: "text",
							text: "Local, private, and link-local network access is disabled by the sandbox policy.",
						},
					],
					details: { granted: false, reason: "network-disabled" },
					isError: true,
				};
			}
			if (
				permission.kind === "local_network" &&
				((sandboxState.config.network?.allowLocalNetwork ?? false) ||
					[...persistentPermissions, ...oneShotPermissions].some(
						(entry) => entry.kind === "local_network",
					))
			) {
				return {
					content: [
						{
							type: "text",
							text: "Localhost, private-network, and link-local access is already granted.",
						},
					],
					details: { granted: true, existing: true },
				};
			}
			if (!ctx.hasUI) {
				return {
					content: [{ type: "text", text: "The user cannot approve IO rights in this mode." }],
					details: { granted: false, reason: "no-ui" },
					isError: true,
				};
			}

			const label =
				params.kind === "local_port"
					? `${permissionLabel(permission)} (requested port ${params.port})`
					: permissionLabel(permission);
			pi.events.emit("approval:requested", {
				kind: "io-permission",
				title: "Agent requests an IO right",
				summary: `${label}\nReason: ${params.reason}`,
				toolName: "request_io_permission",
				toolCallId,
				sessionId: ctx.sessionManager.getSessionId(),
				cwd: ctx.cwd,
			});
			const selection = await ctx.ui.select(
				`Allow ${label}?\n\nReason: ${params.reason}`,
				["Allow once", "Always allow in this workspace", "No", "No, with comment"],
			);
			let comment: string | undefined;
			if (selection === "No, with comment") {
				comment = await ctx.ui.input("Tell the agent what to do instead", "Short note");
			}
			const allow = selection === "Allow once" || selection === "Always allow in this workspace";
			if (selection === "Allow once") {
				oneShotPermissions.push(permission);
			}
			if (selection === "Always allow in this workspace") {
				saveWorkspacePermission(permissionFile, ctx.cwd, permission);
				persistentPermissions = loadWorkspacePermissions(permissionFile, ctx.cwd);
			}
			pi.events.emit("approval:resolved", {
				kind: "io-permission",
				toolName: "request_io_permission",
				toolCallId,
				decision: allow ? "allowed" : "denied",
			});
			return {
				content: [
					{
						type: "text",
						text: allow
							? `${label} granted ${selection === "Allow once" ? "once" : "for this workspace"}.`
							: comment
								? `Permission denied. User comment: ${comment}`
								: "Permission denied.",
					},
				],
				details: {
					granted: allow,
					scope: selection === "Allow once" ? "once" : selection === "Always allow in this workspace" ? "workspace" : "none",
					comment,
				},
				isError: !allow,
			};
		},
	});

	pi.registerTool({
		...localBash,
		label: "bash (Codex sandbox)",
		async execute(id, params, signal, onUpdate) {
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
			oneShotPermissions = [];
			return createBashTool(localCwd, {
				operations: createCodexSandboxOps(sandboxState.config, grants),
			}).execute(id, params, signal, onUpdate);
		},
	});

	pi.on("tool_call", async (event, ctx) => {
		if (event.toolName === "bash" || event.toolName === "request_io_permission") return;
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
		const permissions = [...persistentPermissions, ...oneShotPermissions];
		const matchingIndex = permissions.findIndex(
			(permission) =>
				permission.kind === access && permissionCoversPath(permission, path),
		);
		const allowed =
			matchingIndex >= 0 ||
			(access === "read"
				? isBaseReadAllowed(path, activeConfig(sandboxState), ctx.cwd)
				: isBaseWriteAllowed(path, activeConfig(sandboxState), ctx.cwd));
		if (!allowed) {
			return {
				block: true,
				reason:
					`${access} access is outside the current IO rights: ${path}. ` +
					"Request the exact file or folder with request_io_permission.",
			};
		}
		if (matchingIndex >= persistentPermissions.length) {
			oneShotPermissions.splice(matchingIndex - persistentPermissions.length, 1);
		}
		if ("path" in event.input && typeof event.input.path === "string") {
			event.input.path = path;
		}
	});

	pi.on("user_bash", () => {
		if (sandboxState.kind === "disabled") return;
		if (sandboxState.kind === "ready") {
			return { operations: createCodexSandboxOps(sandboxState.config, runtimeGrants(false)) };
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
		oneShotPermissions = [];
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
		oneShotPermissions = [];
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
