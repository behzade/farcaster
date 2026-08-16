/** Native command sandbox with checked-in project access policy. */

import { randomUUID } from "node:crypto";
import { existsSync, readFileSync, statSync } from "node:fs";
import { basename, dirname, resolve } from "node:path";
import { Effect } from "effect";
import type {
	ExtensionAPI,
	ExtensionContext,
	ToolCallEvent,
} from "@earendil-works/pi-coding-agent";
import {
	type BashOperations,
	createBashTool,
	getAgentDir,
} from "@earendil-works/pi-coding-agent";
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
	type NativeSandboxConfig,
	mergeGlobalConfig,
	normalizeConfig,
} from "./sandbox-config.ts";
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
	resolvePermissionPath,
} from "./io-permissions.ts";
import {
	isBaseReadAllowed,
	isBaseWriteAllowed,
	isDeniedByConfig,
} from "./io-policy.ts";
import { createNativeSandboxOps } from "./native-sandbox-ops.ts";
import { runtimeNetworkHosts } from "./network-policy.ts";
import { requestUserApproval } from "./permission-system-approval.ts";
import { backgroundKeyBytes, NativeBackgroundJobs } from "./native-background-jobs.ts";
import {
	BackgroundJobParams,
	BashParams,
	RequestAccessParams,
} from "./tool-schemas.ts";
import {
	activateProjectPolicy,
	addProjectAccess,
	EMPTY_PROJECT_POLICY,
	loadProjectPolicy,
	loadProjectPolicyForUpdate,
	projectPolicyDiff,
	projectPolicyPath,
	saveProjectPolicy,
	type ActiveProjectPolicy,
	type ProjectAccessRequest,
	type ProjectAccessRight,
} from "./project-policy.ts";

const PACKAGED_BROKER_PATH = "@PI_SANDBOX_BROKER@/bin/pi-sandbox-broker";

function readGlobalConfig(): NativeSandboxConfig {
	const path = resolve(getAgentDir(), "extensions", "sandbox.json");
	if (!existsSync(path)) return mergeGlobalConfig(DEFAULT_CONFIG, {});
	const parsed: unknown = JSON.parse(readFileSync(path, "utf8"));
	return mergeGlobalConfig(DEFAULT_CONFIG, normalizeConfig(parsed));
}

function unavailableBashOps(reason: string): BashOperations {
	return { async exec() { throw new Error(reason); } };
}

type SandboxState =
	| { kind: "disabled"; reason: string }
	| { kind: "initializing" }
	| { kind: "ready"; config: NativeSandboxConfig; machineConfig: NativeSandboxConfig }
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
	let activeProject: ActiveProjectPolicy | undefined;
	let activeProjectCwd = localCwd;
	let brokerClient: SandboxBrokerClient | undefined;
	let backgroundJobs: NativeBackgroundJobs | undefined;
	let userBashCounter = 0;
	let sessionGeneration = 0;

	const revalidateProject = (
		project: ActiveProjectPolicy = requireActiveProject(activeProject),
	): ActiveProjectPolicy => {
		if (sandboxState.kind !== "ready") throw new Error("Sandbox is not ready");
		return activateProjectPolicy(
			project.policy,
			activeProjectCwd,
			sandboxState.machineConfig,
			project.sourceText,
		);
	};
	const networkHosts = (project: ActiveProjectPolicy = requireActiveProject(activeProject)) =>
		runtimeNetworkHosts(project.config, project.networkHosts);

	pi.registerTool({
		name: "request_access",
		label: "Request project access",
		description:
			"Ask the user to add a batch of portable filesystem, network, and/or managed development-cache adapter entries to checked-in .pi/extensions/sandbox/sandbox.json. This host tool updates policy only; it never runs or retries a command.",
		promptSnippet:
			"After a sandbox denial, use request_access for the smallest useful project/home file tree, exact host, local network, or development_cache environment mapping. If approved, explicitly rerun later.",
		parameters: RequestAccessParams,
		executionMode: "sequential",
		async execute(toolCallId, params, signal, _onUpdate, ctx) {
			if (sandboxState.kind !== "ready" || !activeProject) {
				return accessError("The sandbox is not ready, so project policy was not changed.", "sandbox-not-ready");
			}
			if (!ctx.isProjectTrusted()) {
				return accessError("Project access policy can be changed only for a trusted project.", "project-untrusted");
			}
			let diskProject: ActiveProjectPolicy;
			let candidate: ActiveProjectPolicy;
			try {
				diskProject = loadProjectPolicyForUpdate(
					ctx.cwd,
					sandboxState.machineConfig,
					activeProject,
				);
				candidate = addProjectAccess(
					diskProject.policy,
					params.rights as ProjectAccessRequest[],
					ctx.cwd,
					sandboxState.machineConfig,
				);
			} catch (error) {
				return accessError(errorMessage(error), "invalid-or-stale-policy");
			}
			if (JSON.stringify(candidate.policy) === JSON.stringify(diskProject.policy)) {
				return {
					content: [{ type: "text", text: `All requested rights are already active in ${projectPolicyPath(ctx.cwd)}. No command was retried.` }],
					details: { granted: true, existing: true, policyPath: projectPolicyPath(ctx.cwd), commandRetried: false },
				};
			}
			const sourceSnapshot = diskProject.sourceText;
			const diff = projectPolicyDiff(diskProject.policy, candidate.policy, ctx.cwd);
			pi.events.emit("approval:requested", {
				kind: "io-permission",
				title: "Add rights to project sandbox policy",
				summary: diff,
				toolName: "request_access",
				toolCallId,
				sessionId: ctx.sessionManager.getSessionId(),
				cwd: ctx.cwd,
			});
			let approvalDecision: "allowed" | "denied" = "denied";
			const result = await Effect.runPromise(requestUserApproval(ctx, {
				requestId: toolCallId,
				title: "Add rights to project sandbox policy",
				message: `${diff}\n\nReason: ${params.reason}`,
				source: "tool_call",
				surface: "project_policy",
				value: projectPolicyPath(ctx.cwd),
				choices: [
					{ id: "add", label: "Add to project policy" },
					{ id: "deny", label: "Deny" },
				],
				signal,
			}).pipe(
				Effect.tap((value) => Effect.sync(() => {
					approvalDecision = value.choiceId === "add" ? "allowed" : "denied";
				})),
				Effect.ensuring(Effect.sync(() => pi.events.emit("approval:resolved", {
					kind: "io-permission",
					toolName: "request_access",
					toolCallId,
					decision: approvalDecision,
				}))),
			), { signal });
			const approved = result.choiceId === "add";
			if (!approved) {
				return accessError(result.unavailableReason ?? "Project policy change denied.", "denied");
			}
			try {
				candidate.sourceText = saveProjectPolicy(ctx.cwd, candidate.policy, sourceSnapshot);
				ensureDevelopmentCacheDirectories(candidate.config.developmentCache);
				activeProject = candidate;
				sandboxState = { ...sandboxState, config: candidate.config };
			} catch (error) {
				return accessError(`Project policy was not activated: ${errorMessage(error)}`, "save-failed");
			}
			return {
				content: [{
					type: "text",
					text: `Updated and activated ${projectPolicyPath(ctx.cwd)}. No command was retried; explicitly rerun it in a later tool call.`,
				}],
				details: {
					granted: true,
					policyPath: projectPolicyPath(ctx.cwd),
					requests: params.rights,
					commandRetried: false,
				},
			};
		},
	});

	pi.registerTool({
		name: "background_job",
		label: "Background job",
		description:
			"Start, list, inspect, interact with, or stop a session-scoped long-running command. New jobs use the active .pi/extensions/sandbox/sandbox.json policy captured at start; existing jobs keep their start policy.",
		promptSnippet:
			"Use background_job for long-running servers, watchers, builds, and tests. Use request_access separately if policy must change, then start a new job.",
		parameters: BackgroundJobParams,
		executionMode: "sequential",
		async execute(_toolCallId, params, signal, _onUpdate, ctx) {
			if ("name" in params && !isValidBackgroundJobName(params.name)) {
				return toolError("Job names must start with pi- and use only letters, digits, dots, underscores, or hyphens.");
			}
			if (sandboxState.kind !== "ready" || !backgroundJobs) return toolError("The native sandbox is not ready.");
			try {
				let output: string;
				if (params.action === "start") {
					const cwd = resolvePermissionPath(params.cwd ?? ctx.cwd, ctx.cwd);
					if (!isInside(canonicalize(ctx.cwd), cwd)) throw new Error("Background jobs must start inside the current workspace.");
					if (!existsSync(cwd) || !statSync(cwd).isDirectory()) throw new Error(`Background job directory does not exist: ${cwd}`);
					const projectAtStart = revalidateProject();
					output = await backgroundJobs.start({
						name: params.name,
						command: params.command,
						cwd,
						config: projectAtStart.config,
						permissions: projectAtStart.filesystem,
						revalidatePermissions: () => revalidateProject(projectAtStart).filesystem,
						networkHosts: networkHosts(projectAtStart),
						allowLocalBinding: projectAtStart.allowLocalBinding,
					}, signal);
				} else if (params.action === "list") output = backgroundJobs.list();
				else if (params.action === "status") output = backgroundJobs.status(params.name);
				else if (params.action === "read") output = modelVisibleBackgroundJobOutput("read", backgroundJobs.read(params.name, params.lines ?? 200));
				else if (params.action === "write") output = backgroundJobs.write(params.name, Buffer.from(params.text));
				else if (params.action === "line") output = backgroundJobs.write(params.name, Buffer.from(`${params.text}\n`));
				else if (params.action === "keys") output = backgroundJobs.write(params.name, backgroundKeyBytes(params.keys));
				else output = await backgroundJobs.stop(params.name);
				return { content: [{ type: "text", text: output || "Done" }], details: { action: params.action } };
			} catch (error) {
				return toolError(errorMessage(error));
			}
		},
	});

	pi.registerTool({
		...localBash,
		label: "bash (OS sandbox)",
		description:
			"Execute one bash command with the active checked-in project sandbox policy. The call cannot declare rights and is never automatically retried. Use request_access separately after a denial.",
		promptSnippet:
			"Run once under the active policy. On denial, inspect the bounded summary, request the smallest durable project right with request_access, and explicitly rerun later. Prefer managed development caches over host cache grants.",
		parameters: BashParams,
		executionMode: "sequential",
		renderShell: "self",
		async execute(id, params, signal, onUpdate) {
			if (sandboxState.kind === "disabled") return localBash.execute(id, params, signal, onUpdate);
			if (sandboxState.kind !== "ready") throw new Error(sandboxState.kind === "failed" ? sandboxState.reason : "Sandbox is still initializing; command blocked");
			if (!brokerClient) throw new Error("Native sandbox broker is not ready");
			const projectAtStart = revalidateProject();
			const operations = createNativeSandboxOps(
				brokerClient,
				projectAtStart.config,
				projectAtStart.filesystem,
				networkHosts(projectAtStart),
				id,
				projectAtStart.allowLocalBinding,
				() => revalidateProject(projectAtStart).filesystem,
			);
			return createBashTool(localCwd, { operations }).execute(id, params, signal, onUpdate);
		},
	});

	pi.on("tool_call", async (event, ctx) => {
		if (sandboxState.kind === "disabled") return;
		if (!["read", "write", "edit", "grep", "find", "ls"].includes(event.toolName)) return;
		if (event.toolName === "grep" || event.toolName === "find") {
			return {
				block: true,
				reason: `Use ${event.toolName === "grep" ? "rg" : "fd"} through bash; this recursive host tool cannot enforce the sandbox policy. If bash is denied, use request_access and rerun explicitly.`,
			};
		}
		const lexicalPath = toolLexicalPath(event, ctx.cwd);
		if (!lexicalPath) return { block: true, reason: "File path is missing" };
		const path = canonicalize(lexicalPath);
		const access = event.toolName === "write" || event.toolName === "edit" ? "write" : "read";
		const config = activeConfig(sandboxState);
		if (
			isProtectedPath(lexicalPath) ||
			(access === "write" && isProtectedWritePath(lexicalPath)) ||
			isDeniedByConfig(path, access, config, ctx.cwd)
		) {
			return { block: true, reason: `Protected or machine-denied ${access} path cannot be granted: ${path}` };
		}
		const gitRoot = access === "write" ? gitControlRoot(lexicalPath, ctx.cwd) : undefined;
		const piRoot = access === "write" ? projectControlRoot(lexicalPath, ctx.cwd) : undefined;
		if (piRoot) return { block: true, reason: "Sandboxed tools cannot write project .pi; trusted host tools own that control folder." };
		if (gitRoot && isControlRootSymlink(gitRoot)) {
			return { block: true, reason: `Writes to a symlinked control folder cannot be granted: ${gitRoot}` };
		}
		const controlRoot = gitRoot;
		const fileRights = revalidateProject().filesystem;
		const allowed = controlRoot
			? fileRights.some((permission) =>
				permission.kind === access && permission.directory && lexicalControlKey(permission.path) === lexicalControlKey(controlRoot))
			: fileRights.some((permission) =>
				(permission.kind === access || (access === "read" && permission.kind === "write")) && permissionCoversPath(permission, path)) ||
				(access === "read" ? isBaseReadAllowed(path, config, ctx.cwd) : isBaseWriteAllowed(path, config, ctx.cwd));
		if (!allowed) {
			return {
				block: true,
				reason: `Sandbox policy denied ${access} access to ${controlRoot ?? path}. Use request_access for the smallest file or tree right, then explicitly retry the file tool.`,
			};
		}
		if ("path" in event.input && typeof event.input.path === "string") event.input.path = path;
	});

	pi.on("user_bash", () => {
		if (sandboxState.kind === "disabled") return;
		if (sandboxState.kind === "ready") {
			if (!brokerClient) return { operations: unavailableBashOps("Native sandbox broker is not ready") };
			try {
				const projectAtStart = revalidateProject();
				return {
					operations: createNativeSandboxOps(
						brokerClient,
						projectAtStart.config,
						projectAtStart.filesystem,
						networkHosts(projectAtStart),
						`user-bash-${++userBashCounter}-${randomUUID()}`,
						projectAtStart.allowLocalBinding,
						() => revalidateProject(projectAtStart).filesystem,
					),
				};
			} catch (error) {
				return { operations: unavailableBashOps(errorMessage(error)) };
			}
		}
		return { operations: unavailableBashOps(sandboxState.kind === "failed" ? sandboxState.reason : "Sandbox is still initializing; command blocked") };
	});

	pi.on("session_start", async (_event, ctx) => {
		const generation = ++sessionGeneration;
		if (pi.getFlag("no-sandbox") as boolean) {
			sandboxState = { kind: "disabled", reason: "disabled via --no-sandbox" };
			ctx.ui.notify("Sandbox disabled via --no-sandbox", "warning");
			return;
		}
		try {
			const machineConfig = readGlobalConfig();
			if (!machineConfig.enabled) {
				sandboxState = { kind: "disabled", reason: "disabled via global config" };
				ctx.ui.notify("Sandbox disabled via global config", "warning");
				return;
			}
			activeProjectCwd = ctx.cwd;
			activeProject = ctx.isProjectTrusted()
				? loadProjectPolicy(ctx.cwd, machineConfig)
				: activateProjectPolicy(EMPTY_PROJECT_POLICY, ctx.cwd, machineConfig);
			sandboxState = { kind: "initializing" };
			ensureDevelopmentCacheDirectories(activeProject.config.developmentCache);
			if (process.platform !== "darwin" && process.platform !== "linux") throw new Error("the native sandbox supports macOS and Linux only");
			const brokerPath = machineConfig.brokerPath ?? PACKAGED_BROKER_PATH;
			const client = await SandboxBrokerClient.start(brokerPath);
			if (generation !== sessionGeneration) { await client.shutdown(); return; }
			brokerClient = client;
			backgroundJobs = new NativeBackgroundJobs(brokerPath);
			sandboxState = { kind: "ready", config: activeProject.config, machineConfig };
			const backendLabel = `native ${process.platform === "linux" ? "Bubblewrap" : "Seatbelt"}`;
			ctx.ui.setStatus("sandbox", ctx.ui.theme.fg("accent", `🔒 ${backendLabel}`));
			ctx.ui.notify(`${backendLabel} sandbox ready`, "info");
		} catch (error) {
			if (generation !== sessionGeneration) return;
			const reason = `Sandbox unavailable; commands are blocked: ${errorMessage(error)}`;
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
		activeProject = undefined;
		activeProjectCwd = localCwd;
		userBashCounter = 0;
		sandboxState = { kind: "initializing" };
	});

	pi.registerCommand("sandbox", {
		description: "Show OS sandbox rights",
		handler: async (_args, ctx) => {
			if (sandboxState.kind !== "ready") {
				ctx.ui.notify(sandboxState.kind === "disabled" ? `Sandbox is ${sandboxState.reason}` : sandboxState.kind === "failed" ? sandboxState.reason : "Sandbox is initializing", sandboxState.kind === "failed" ? "error" : "info");
				return;
			}
			ctx.ui.notify([
				"OS sandbox (native-preview):",
				`  Project policy: ${projectPolicyPath(ctx.cwd)}`,
				`  Project rights: ${activeProject?.policy.rights.map(rightLabel).join(", ") || "(none)"}`,
				`  Network hosts: ${networkHosts().join(", ") || "(blocked)"}`,
				`  Local network: ${activeProject?.allowLocalBinding ? "allowed" : "blocked"}`,
				`  Development cache: ${developmentCacheRoot(sandboxState.config.developmentCache)}`,
				"  Denials: bounded diagnostics; no automatic retry",
			].join("\n"), "info");
		},
	});
}

function requireActiveProject(
	project: ActiveProjectPolicy | undefined,
): ActiveProjectPolicy {
	if (!project) throw new Error("Active project sandbox policy is unavailable");
	return project;
}

function accessError(message: string, reason: string) {
	return {
		content: [{ type: "text" as const, text: `${message} No command was retried.` }],
		details: { granted: false, reason, commandRetried: false },
		isError: true,
	};
}

function toolError(message: string) {
	return { content: [{ type: "text" as const, text: message }], isError: true };
}

function errorMessage(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

function activeConfig(state: SandboxState): NativeSandboxConfig {
	return state.kind === "ready" ? state.config : DEFAULT_CONFIG;
}

function lexicalControlKey(path: string): string {
	return resolve(canonicalize(dirname(path)), basename(path));
}

function toolLexicalPath(event: ToolCallEvent, cwd: string): string | undefined {
	if (!("path" in event.input) || event.input.path === undefined) return event.toolName === "ls" ? resolve(cwd) : undefined;
	if (typeof event.input.path !== "string") return undefined;
	return resolveLexicalPermissionPath(event.input.path, cwd);
}

function rightLabel(right: ProjectAccessRight): string {
	if (right.kind === "filesystem") return `${right.access} ${right.scope} ${right.path}`;
	if (right.kind === "network_host") return `host ${right.host}`;
	return "local network";
}
