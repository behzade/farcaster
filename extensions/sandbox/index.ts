/**
 * Sandbox Extension - OS-level sandboxing for bash commands
 *
 * Uses @anthropic-ai/sandbox-runtime to enforce filesystem and network
 * restrictions on bash commands at the OS level (sandbox-exec on macOS,
 * bubblewrap on Linux).
 *
 * Note: this example intentionally overrides the built-in `bash` tool to show
 * how built-in tools can be replaced. Alternatively, you could sandbox `bash`
 * via `tool_call` input mutation without replacing the tool.
 *
 * Config files:
 * - ~/.pi/agent/extensions/sandbox.json (global)
 * - <cwd>/.pi/sandbox.json (trusted project-local restrictions only)
 *
 * Example .pi/sandbox.json:
 * ```json
 * {
 *   "enabled": true,
 *   "network": {
 *     "allowedDomains": ["github.com", "*.github.com"],
 *     "deniedDomains": []
 *   },
 *   "filesystem": {
 *     "denyRead": ["~/.ssh", "~/.aws"],
 *     "allowWrite": [".", "/tmp"],
 *     "denyWrite": [".env"]
 *   }
 * }
 * ```
 *
 * Usage:
 * - `pi -e ./sandbox` - sandbox enabled with default/config settings
 * - `pi -e ./sandbox --no-sandbox` - disable sandboxing
 * - `/sandbox` - show current sandbox configuration
 *
 * Setup:
 * 1. Copy sandbox/ directory to ~/.pi/agent/extensions/
 * 2. Run `npm install` in ~/.pi/agent/extensions/sandbox/
 *
 * Linux also requires: bubblewrap, socat, ripgrep
 */

import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { SandboxManager, type SandboxRuntimeConfig } from "@anthropic-ai/sandbox-runtime";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { type BashOperations, CONFIG_DIR_NAME, createBashTool, getAgentDir } from "@earendil-works/pi-coding-agent";

interface SandboxConfig extends SandboxRuntimeConfig {
	enabled?: boolean;
}

const DEFAULT_CONFIG: SandboxConfig = {
	enabled: true,
	network: {
		allowedDomains: [
			"npmjs.org",
			"*.npmjs.org",
			"registry.npmjs.org",
			"registry.yarnpkg.com",
			"pypi.org",
			"*.pypi.org",
			"github.com",
			"*.github.com",
			"api.github.com",
			"raw.githubusercontent.com",
			"opencode.ai",
			"*.opencode.ai",
			"models.dev",
		],
		deniedDomains: [],
	},
	filesystem: {
		denyRead: ["~/.ssh", "~/.aws", "~/.gnupg"],
		allowWrite: [".", "/tmp"],
		denyWrite: [".env", ".env.*", "*.pem", "*.key"],
	},
};

function loadConfig(cwd: string, projectTrusted: boolean): SandboxConfig {
	const projectConfigPath = join(cwd, CONFIG_DIR_NAME, "sandbox.json");
	const globalConfigPath = join(getAgentDir(), "extensions", "sandbox.json");

	let globalConfig: Partial<SandboxConfig> = {};
	let projectConfig: Partial<SandboxConfig> = {};

	if (existsSync(globalConfigPath)) {
		try {
			globalConfig = JSON.parse(readFileSync(globalConfigPath, "utf-8"));
		} catch (e) {
			console.error(`Warning: Could not parse ${globalConfigPath}: ${e}`);
		}
	}

	if (projectTrusted && existsSync(projectConfigPath)) {
		try {
			projectConfig = JSON.parse(readFileSync(projectConfigPath, "utf-8"));
		} catch (e) {
			console.error(`Warning: Could not parse ${projectConfigPath}: ${e}`);
		}
	}

	return applyProjectRestrictions(deepMerge(DEFAULT_CONFIG, globalConfig), projectConfig);
}

function applyProjectRestrictions(base: SandboxConfig, project: Partial<SandboxConfig>): SandboxConfig {
	const result = deepMerge(base, {});
	if (project.allowPty === false) result.allowPty = false;
	if (project.network?.deniedDomains) {
		result.network = {
			...result.network,
			deniedDomains: unique([
				...(result.network?.deniedDomains ?? []),
				...project.network.deniedDomains,
			]),
		};
	}
	if (project.filesystem?.denyRead || project.filesystem?.denyWrite) {
		result.filesystem = {
			...result.filesystem,
			denyRead: unique([
				...(result.filesystem?.denyRead ?? []),
				...(project.filesystem.denyRead ?? []),
			]),
			denyWrite: unique([
				...(result.filesystem?.denyWrite ?? []),
				...(project.filesystem.denyWrite ?? []),
			]),
		};
	}
	const projectExt = project as { enableWeakerNestedSandbox?: boolean };
	if (projectExt.enableWeakerNestedSandbox === false) {
		(result as { enableWeakerNestedSandbox?: boolean }).enableWeakerNestedSandbox = false;
	}
	return result;
}

function unique(values: string[]): string[] {
	return [...new Set(values)];
}

function deepMerge(base: SandboxConfig, overrides: Partial<SandboxConfig>): SandboxConfig {
	const result: SandboxConfig = { ...base };

	if (overrides.enabled !== undefined) result.enabled = overrides.enabled;
	if (overrides.allowPty !== undefined) result.allowPty = overrides.allowPty;
	if (overrides.network) {
		result.network = { ...base.network, ...overrides.network };
	}
	if (overrides.filesystem) {
		result.filesystem = { ...base.filesystem, ...overrides.filesystem };
	}

	const extOverrides = overrides as {
		ignoreViolations?: Record<string, string[]>;
		enableWeakerNestedSandbox?: boolean;
	};
	const extResult = result as { ignoreViolations?: Record<string, string[]>; enableWeakerNestedSandbox?: boolean };

	if (extOverrides.ignoreViolations) {
		extResult.ignoreViolations = extOverrides.ignoreViolations;
	}
	if (extOverrides.enableWeakerNestedSandbox !== undefined) {
		extResult.enableWeakerNestedSandbox = extOverrides.enableWeakerNestedSandbox;
	}

	return result;
}

function createSandboxedBashOps(baseAllowWrite: string[], extraAllowWrite: string[] = []): BashOperations {
	return {
		async exec(command, cwd, { onData, signal, timeout }) {
			if (!existsSync(cwd)) {
				throw new Error(`Working directory does not exist: ${cwd}`);
			}

			const allowWrite = unique([...baseAllowWrite, ...extraAllowWrite]);
			const customConfig = allowWrite.length
				? ({
						filesystem: { allowWrite },
					} as Partial<SandboxRuntimeConfig>)
				: undefined;
			const wrappedCommand = await SandboxManager.wrapWithSandbox(
				command,
				undefined,
				customConfig,
				signal,
			);

			return new Promise((resolve, reject) => {
				const child = spawn("bash", ["-c", wrappedCommand], {
					cwd,
					detached: true,
					env: {
						...process.env,
						IN_SANDBOX: "1",
						PI_SANDBOX: process.platform === "darwin" ? "seatbelt" : "bubblewrap",
						PI_SANDBOX_NETWORK_RESTRICTED: "1",
					},
					stdio: ["ignore", "pipe", "pipe"],
				});

				let timedOut = false;
				let timeoutHandle: NodeJS.Timeout | undefined;

				if (timeout !== undefined && timeout > 0) {
					timeoutHandle = setTimeout(() => {
						timedOut = true;
						if (child.pid) {
							try {
								process.kill(-child.pid, "SIGKILL");
							} catch {
								child.kill("SIGKILL");
							}
						}
					}, timeout * 1000);
				}

				child.stdout?.on("data", onData);
				child.stderr?.on("data", onData);

				child.on("error", (err) => {
					if (timeoutHandle) clearTimeout(timeoutHandle);
					reject(err);
				});

				const onAbort = () => {
					if (child.pid) {
						try {
							process.kill(-child.pid, "SIGKILL");
						} catch {
							child.kill("SIGKILL");
						}
					}
				};

				signal?.addEventListener("abort", onAbort, { once: true });

				child.on("close", (code) => {
					if (timeoutHandle) clearTimeout(timeoutHandle);
					signal?.removeEventListener("abort", onAbort);

					if (signal?.aborted) {
						reject(new Error("aborted"));
					} else if (timedOut) {
						reject(new Error(`timeout:${timeout}`));
					} else {
						resolve({ exitCode: code });
					}
				});
			});
		},
	};
}

function createUnavailableBashOps(reason: string): BashOperations {
	return {
		async exec() {
			throw new Error(reason);
		},
	};
}

type SandboxState =
	| { kind: "disabled"; reason: string }
	| { kind: "initializing" }
	| { kind: "ready"; config: SandboxConfig }
	| { kind: "failed"; reason: string };

export default function (pi: ExtensionAPI) {
	pi.registerFlag("no-sandbox", {
		description: "Disable OS-level sandboxing for bash commands",
		type: "boolean",
		default: false,
	});

	const localCwd = process.cwd();
	const localBash = createBashTool(localCwd);

	let sandboxState: SandboxState = { kind: "initializing" };
	const oneShotWriteGrants = new Map<string, string[]>();
	let unsubscribeGuardian: (() => void) | undefined;

	const subscribeGuardian = () => {
		unsubscribeGuardian?.();
		unsubscribeGuardian = pi.events.on("guardian:sandbox-allow-once", (data: unknown) => {
			if (!data || typeof data !== "object") return;
			const grant = data as { toolCallId?: unknown; allowWrite?: unknown };
			if (typeof grant.toolCallId !== "string" || !Array.isArray(grant.allowWrite)) return;
			const paths = grant.allowWrite.filter(
				(path): path is string => typeof path === "string" && path.length > 0,
			);
			if (paths.length > 0) oneShotWriteGrants.set(grant.toolCallId, paths);
		});
	};

	pi.registerTool({
		...localBash,
		label: "bash (sandboxed)",
		async execute(id, params, signal, onUpdate, _ctx) {
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

			const allowWrite = oneShotWriteGrants.get(id);
			oneShotWriteGrants.delete(id);
			const sandboxedBash = createBashTool(localCwd, {
				operations: createSandboxedBashOps(
					sandboxState.config.filesystem?.allowWrite ?? [],
					allowWrite ?? [],
				),
			});
			return sandboxedBash.execute(id, params, signal, onUpdate);
		},
	});

	pi.on("user_bash", () => {
		if (sandboxState.kind === "disabled") return;
		if (sandboxState.kind === "ready") {
			return {
				operations: createSandboxedBashOps(
					sandboxState.config.filesystem?.allowWrite ?? [],
				),
			};
		}
		return {
			operations: createUnavailableBashOps(
				sandboxState.kind === "failed"
					? sandboxState.reason
					: "Sandbox is still initializing; command blocked",
			),
		};
	});

	pi.on("session_start", async (_event, ctx) => {
		subscribeGuardian();
		oneShotWriteGrants.clear();
		const noSandbox = pi.getFlag("no-sandbox") as boolean;

		if (noSandbox) {
			sandboxState = { kind: "disabled", reason: "disabled via --no-sandbox" };
			ctx.ui.notify("Sandbox disabled via --no-sandbox", "warning");
			return;
		}

		const config = loadConfig(ctx.cwd, ctx.isProjectTrusted());

		if (!config.enabled) {
			sandboxState = { kind: "disabled", reason: "disabled via global config" };
			ctx.ui.notify("Sandbox disabled via config", "info");
			return;
		}

		const platform = process.platform;
		if (platform !== "darwin" && platform !== "linux") {
			const reason = `Sandbox is not supported on ${platform}; commands are blocked`;
			sandboxState = { kind: "failed", reason };
			ctx.ui.notify(reason, "error");
			return;
		}

		sandboxState = { kind: "initializing" };
		try {
			const configExt = config as unknown as {
				ignoreViolations?: Record<string, string[]>;
				enableWeakerNestedSandbox?: boolean;
			};

			await SandboxManager.initialize({
				network: config.network,
				filesystem: config.filesystem,
				allowPty: config.allowPty,
				ignoreViolations: configExt.ignoreViolations,
				enableWeakerNestedSandbox: configExt.enableWeakerNestedSandbox,
			});

			sandboxState = { kind: "ready", config };

			const networkCount = config.network?.allowedDomains?.length ?? 0;
			const writeCount = config.filesystem?.allowWrite?.length ?? 0;
			ctx.ui.setStatus(
				"sandbox",
				ctx.ui.theme.fg("accent", `🔒 Sandbox: ${networkCount} domains, ${writeCount} write paths`),
			);
			ctx.ui.notify("Sandbox initialized", "info");
		} catch (err) {
			const reason = `Sandbox initialization failed; commands are blocked: ${
				err instanceof Error ? err.message : err
			}`;
			sandboxState = { kind: "failed", reason };
			ctx.ui.notify(reason, "error");
		}
	});

	pi.on("session_shutdown", async () => {
		oneShotWriteGrants.clear();
		unsubscribeGuardian?.();
		unsubscribeGuardian = undefined;
		if (sandboxState.kind === "ready") {
			try {
				await SandboxManager.reset();
			} catch {
				// Ignore cleanup errors
			}
		}
		sandboxState = { kind: "initializing" };
	});

	pi.registerCommand("sandbox", {
		description: "Show sandbox configuration",
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
			const lines = [
				"Sandbox Configuration:",
				"",
				"Network:",
				`  Allowed: ${config.network?.allowedDomains?.join(", ") || "(none)"}`,
				`  Denied: ${config.network?.deniedDomains?.join(", ") || "(none)"}`,
				"",
				"Filesystem:",
				`  Deny Read: ${config.filesystem?.denyRead?.join(", ") || "(none)"}`,
				`  Allow Write: ${config.filesystem?.allowWrite?.join(", ") || "(none)"}`,
				`  Deny Write: ${config.filesystem?.denyWrite?.join(", ") || "(none)"}`,
			];
			ctx.ui.notify(lines.join("\n"), "info");
		},
	});
}
