import type { AgentMessage } from "@earendil-works/pi-agent-core";
import { convertToLlm } from "@earendil-works/pi-coding-agent";
import type { ExtensionAPI, ExtensionContext, ToolCallEvent } from "@earendil-works/pi-coding-agent";
import type { Message } from "@earendil-works/pi-ai";
import { randomUUID } from "node:crypto";
import {
	SESSION_ENTRY_TYPE,
	formatApprovalMode,
	formatContextMessages,
	formatModelSetting,
	loadPersistentConfig,
	loadSessionOverrides,
	parseApprovalMode,
	parseContextValue,
	parseModelRef,
	savePersistentConfig,
} from "./config.js";
import { createDebugLogger } from "./logger.js";
import type {
	ConfigScope,
	DebugLogger,
	PreflightConfig,
	ToolDecision,
	ToolPolicyDecision,
	ToolPreflightMetadata,
	ToolCallSummary,
	ToolCallsContext,
} from "./types.js";
import { notify } from "./ui.js";
import { buildPreflightMetadata, findExplicitApprovalDirective } from "./preflight.js";
import { handlePreflightFailure } from "./approvals/failure.js";
import { collectApprovals } from "./approvals/index.js";
import { loadPermissionsState } from "./permissions/state.js";
import { resolveDeterministicDecisionForToolCall, resolveToolDecisions } from "./permissions/decisions.js";
import { getPolicyRulesForTool } from "./permissions/matching.js";
import { gateToolCall, loadCommandPolicy } from "./command-gate.js";
import {
	RepeatTracker,
	buildActionFingerprint,
	hasProjectGuardianRule,
	persistProjectGuardianRule,
} from "./guardian-rules.js";

let persistentConfig = loadPersistentConfig();
let sessionOverride: Partial<PreflightConfig> = {};
let lastContextSnapshot: AgentMessage[] | undefined;
const repeatTracker = new RepeatTracker();
let commandPolicy = loadCommandPolicy();
interface CompletedBatchReview {
	preflight: Record<string, ToolPreflightMetadata>;
	decisions: Record<string, ToolDecision>;
	approvals: Record<string, { allow: boolean; reason?: string }> | undefined;
}
interface CachedBatchReview {
	inputFingerprint: string;
	review: CompletedBatchReview;
}
const completedBatchReviews = new Map<string, CachedBatchReview>();

export default function (pi: ExtensionAPI) {
	pi.on("session_start", (_event, ctx) => {
		refreshConfigs(ctx);
	});

	pi.on("context", (event) => {
		lastContextSnapshot = [...event.messages];
	});

	pi.on("turn_end", () => {
		completedBatchReviews.clear();
	});

	pi.registerCommand("preflight", {
		description: "Configure tool preflight approvals",
		handler: async (args, ctx) => {
			await handlePreflightCommand(args, ctx, pi);
		},
	});

	pi.on("tool_call", async (event, ctx) => {
		const activeConfig = getActiveConfig();
		const logDebug = createDebugLogger(ctx, activeConfig);
		let gate = gateToolCall(event, ctx, commandPolicy);
		if (gate.action === "block") {
			return { block: true, reason: gate.reason };
		}
		const concurrentRace = findConcurrentFileRace(event, ctx, logDebug);
		if (concurrentRace) {
			return { block: true, reason: concurrentRace };
		}

		const toolCall = toToolCallSummary(event);
		let toolCalls = [toolCall];
		let preflightEvent = buildToolCallsContext(toolCalls, ctx, logDebug);
		const explicitUserDirective = findExplicitApprovalDirective(preflightEvent.llmContext.messages);
		if (explicitUserDirective === "mixed") {
			toolCalls = resolveSiblingToolCalls(event, ctx, logDebug);
			preflightEvent = buildToolCallsContext(toolCalls, ctx, logDebug);
		}
		if (gate.action === "allow" && !explicitUserDirective) return undefined;
		if (gate.action === "allow") {
			gate = {
				action: "review",
				reason: `Latest user requested an explicit ${explicitUserDirective} approval verdict`,
			};
		}

		const action = buildActionFingerprint(toolCall, ctx.cwd);
		if (
			!explicitUserDirective &&
			hasProjectGuardianRule(ctx.cwd, action, logDebug, ctx.isProjectTrusted())
		) {
			logDebug(`Project guardian rule matched: ${action.label}`);
			grantSandboxAccess(pi, event, gate);
			return undefined;
		}

		const permissions = loadPermissionsState(ctx.cwd, logDebug, ctx.isProjectTrusted());
		const hasRules =
			permissions.rules.allow.length > 0 ||
			permissions.rules.ask.length > 0 ||
			permissions.rules.deny.length > 0 ||
			permissions.policyRules.length > 0;
		if (activeConfig.approvalMode === "off" && !hasRules && !explicitUserDirective) {
			return undefined;
		}

		const deterministic = resolveDeterministicDecisionForToolCall(toolCall, ctx.cwd, permissions, logDebug);
		if (deterministic?.decision === "deny") {
			const reason = deterministic.reason ?? "Blocked by deterministic rule.";
			return { block: true, reason };
		}
		if (deterministic?.decision === "allow" && !explicitUserDirective) {
			logDebug(
				`Skipping preflight LLM for ${toolCall.name}: deterministic ${deterministic.decision} decision already resolved.`,
			);
			grantSandboxAccess(pi, event, gate);
			return undefined;
		}

		const cachedCandidate = completedBatchReviews.get(toolCall.id);
		completedBatchReviews.delete(toolCall.id);
		const cachedBatch =
			cachedCandidate &&
			matchesReviewedInput(toolCall, ctx.cwd, cachedCandidate.inputFingerprint)
				? cachedCandidate.review
				: undefined;
		if (cachedCandidate && !cachedBatch) {
			logDebug(`Discarding stale sibling review for ${toolCall.name}: live arguments changed.`);
			toolCalls = [toolCall];
			preflightEvent = buildToolCallsContext(toolCalls, ctx, logDebug);
		}
		let preflight: Record<string, ToolPreflightMetadata>;
		let decisions: Record<string, ToolDecision>;
		let approvals: Record<string, { allow: boolean; reason?: string }> | undefined;

		if (cachedBatch) {
			({ preflight, decisions, approvals } = cachedBatch);
			logDebug(`Using cached sibling review for ${toolCall.name}.`);
		} else {
			const policyRulesByToolCall: Record<string, string[]> = {};
			for (const pendingToolCall of toolCalls) {
				policyRulesByToolCall[pendingToolCall.id] = getPolicyRulesForTool(
					pendingToolCall.name,
					permissions.policyRules,
				);
			}

			logDebug(`Preflight tool call batch: ${toolCalls.map((call) => call.name).join(", ")}.`);

			let preflightResult = await buildPreflightMetadata(
				preflightEvent,
				policyRulesByToolCall,
				ctx,
				activeConfig,
				logDebug,
			);
			while (preflightResult.status === "error") {
				const decision = await handlePreflightFailure(toolCalls, preflightResult.reason, ctx, logDebug);
				if (decision.action === "retry") {
					preflightResult = await buildPreflightMetadata(
						preflightEvent,
						policyRulesByToolCall,
						ctx,
						activeConfig,
						logDebug,
					);
					continue;
				}
				if (decision.action === "allow") {
					grantSandboxAccess(pi, event, gate);
					return undefined;
				}
				return { block: true, reason: decision.reason };
			}

			preflight = preflightResult.metadata;
			const policyDecisions: Record<string, ToolPolicyDecision> = preflightResult.policyDecisions;
			decisions = await resolveToolDecisions(
				preflightEvent,
				preflight,
				policyDecisions,
				ctx,
				activeConfig,
				permissions,
				logDebug,
			);
			approvals = await collectApprovals(
				preflightEvent,
				preflight,
				decisions,
				ctx,
				activeConfig,
				logDebug,
			);
			if (approvals && Object.keys(approvals).length > 0) {
				logDebug(`Preflight approvals collected for ${Object.keys(approvals).length} tool call(s).`);
			}

			if (toolCalls.length > 1) {
				const completed = { preflight, decisions, approvals };
				for (const pendingToolCall of toolCalls) {
					if (pendingToolCall.id !== toolCall.id) {
						completedBatchReviews.set(pendingToolCall.id, {
							inputFingerprint: buildActionFingerprint(pendingToolCall, ctx.cwd).fingerprint,
							review: completed,
						});
					}
				}
			}
		}

		const approval = approvals?.[toolCall.id];
		if (approval) {
			if (!approval.allow) {
				const reason = approval.reason ?? "Blocked by user.";
				return { block: true, reason };
			}
			return finalizeAllowedToolCall(pi, event, ctx, activeConfig, action, gate, logDebug);
		}

		const decision = decisions[toolCall.id];
		if (decision?.decision === "deny") {
			const reason = decision.reason ?? "Blocked by policy.";
			if (ctx.hasUI) ctx.ui.notify(`Guardian blocked: ${reason}`, "warning");
			return { block: true, reason };
		}

		return finalizeAllowedToolCall(pi, event, ctx, activeConfig, action, gate, logDebug);
	});

}

function refreshConfigs(ctx: ExtensionContext): void {
	persistentConfig = loadPersistentConfig();
	sessionOverride = loadSessionOverrides(ctx);
	lastContextSnapshot = undefined;
	repeatTracker.clear();
	completedBatchReviews.clear();
	commandPolicy = loadCommandPolicy();
}

function getActiveConfig(): PreflightConfig {
	return { ...persistentConfig, ...sessionOverride };
}

function getConfigForScope(scope: ConfigScope): PreflightConfig {
	if (scope === "persistent") {
		return { ...persistentConfig };
	}
	return { ...persistentConfig, ...sessionOverride };
}

async function handlePreflightCommand(args: string, ctx: ExtensionContext, pi: ExtensionAPI): Promise<void> {
	const trimmed = args.trim();
	if (!trimmed) {
		if (!ctx.hasUI) return;
		await openConfigMenu(ctx, pi);
		return;
	}

	const parts = trimmed.split(/\s+/);
	const action = parts[0]?.toLowerCase();
	const scope = parseScope(parts);

	if (action === "status") {
		showStatus(ctx, getActiveConfig());
		return;
	}

	if (action === "on" || action === "off") {
		applyConfig({ approvalMode: action === "on" ? "all" : "off" }, scope, pi, ctx);
		showStatus(ctx, getActiveConfig());
		return;
	}

	if (action === "context") {
		const rawValue = parts.slice(1).join(" ");
		const parsed = parseContextValue(rawValue);
		if (parsed === undefined) {
			notify(ctx, "Invalid explain context value. Use 'full' or a positive number.");
			return;
		}
		applyConfig({ contextMessages: parsed }, scope, pi, ctx);
		showStatus(ctx, getActiveConfig());
		return;
	}

	if (action === "model") {
		const modelRef = parseModelRef(parts.slice(1));
		if (!modelRef) {
			notify(ctx, "Invalid model. Use 'current' or 'provider/model-id'.");
			return;
		}
		applyConfig({ model: modelRef }, scope, pi, ctx);
		showStatus(ctx, getActiveConfig());
		return;
	}

	if (action === "policy-model") {
		const modelRef = parseModelRef(parts.slice(1));
		if (!modelRef) {
			notify(ctx, "Invalid policy model. Use 'current' or 'provider/model-id'.");
			return;
		}
		applyConfig({ policyModel: modelRef }, scope, pi, ctx);
		showStatus(ctx, getActiveConfig());
		return;
	}

	if (action === "approvals" || action === "approval") {
		const mode = parseApprovalMode(parts.slice(1));
		if (!mode) {
			notify(ctx, "Invalid mode. Use 'auto', 'all', 'destructive', or 'off'.");
			return;
		}
		applyConfig({ approvalMode: mode }, scope, pi, ctx);
		showStatus(ctx, getActiveConfig());
		return;
	}

	if (action === "destructive-only") {
		const setting = parts[1]?.toLowerCase();
		if (setting !== "on" && setting !== "off") {
			notify(ctx, "Invalid approval setting. Use 'on' or 'off'.");
			return;
		}
		applyConfig({ approvalMode: setting === "on" ? "destructive" : "all" }, scope, pi, ctx);
		showStatus(ctx, getActiveConfig());
		return;
	}

	if (action === "debug") {
		const setting = parts[1]?.toLowerCase();
		if (setting !== "on" && setting !== "off") {
			notify(ctx, "Invalid debug setting. Use 'on' or 'off'.");
			return;
		}
		applyConfig({ debug: setting === "on" }, scope, pi, ctx);
		showStatus(ctx, getActiveConfig());
		return;
	}

	if (action === "reset-session") {
		clearSessionOverrides(pi);
		showStatus(ctx, getActiveConfig());
		return;
	}

	notify(ctx, "Unknown command. Use /preflight for the interactive menu.");
}

async function openConfigMenu(ctx: ExtensionContext, pi: ExtensionAPI): Promise<void> {
	if (!ctx.hasUI) return;

	while (true) {
		const options = [
			{
				key: "session",
				label: sessionOverrideExists()
					? "Session settings (overrides active)"
					: "Session settings",
			},
			{
				key: "persistent",
				label: "Default settings",
			},
			{
				key: "clear-session",
				label: sessionOverrideExists()
					? "Clear session overrides"
					: "Clear session overrides (none)",
			},
			{
				key: "status",
				label: "Show status",
			},
			{
				key: "exit",
				label: "Exit",
			},
		];

		const selection = await ctx.ui.select("Preflight settings", options.map((option) => option.label));
		if (!selection) return;

		const selected = options.find((option) => option.label === selection);
		if (!selected) return;

		switch (selected.key) {
			case "session":
				await openScopedConfigMenu(ctx, pi, "session");
				break;
			case "persistent":
				await openScopedConfigMenu(ctx, pi, "persistent");
				break;
			case "clear-session": {
				if (!sessionOverrideExists()) {
					notify(ctx, "No session overrides to clear.");
					break;
				}
				const confirm = await ctx.ui.confirm("Clear session overrides", "Reset session-only settings?");
				if (!confirm) break;
				clearSessionOverrides(pi);
				showStatus(ctx, getActiveConfig());
				break;
			}
			case "status":
				showStatus(ctx, getActiveConfig());
				break;
			case "exit":
				return;
			default:
				return;
		}
	}
}

async function openScopedConfigMenu(
	ctx: ExtensionContext,
	pi: ExtensionAPI,
	scope: ConfigScope,
): Promise<void> {
	if (!ctx.hasUI) return;

	const title = scope === "session" ? "Session settings" : "Default settings";

	while (true) {
		const scopedConfig = getConfigForScope(scope);
		const options = [
			{
				key: "mode",
				label: `Mode: ${formatApprovalMode(scopedConfig.approvalMode)}`,
			},
			{
				key: "context",
				label: `Explain context: ${formatContextMessages(scopedConfig.contextMessages)}`,
			},
			{
				key: "model",
				label: `Model: ${formatModelSetting(scopedConfig.model, ctx.model)}`,
			},
			{
				key: "policy-model",
				label: `Policy model: ${formatModelSetting(scopedConfig.policyModel, ctx.model)}`,
			},
			{
				key: "debug",
				label: `Debug: ${scopedConfig.debug ? "on" : "off"}`,
			},
			{
				key: "back",
				label: "Back",
			},
		];

		const selection = await ctx.ui.select(title, options.map((option) => option.label));
		if (!selection) return;

		const selected = options.find((option) => option.label === selection);
		if (!selected) return;

		switch (selected.key) {
			case "mode": {
				const mode = await chooseApprovalMode(ctx, scopedConfig.approvalMode);
				if (!mode) return;
				applyConfig({ approvalMode: mode }, scope, pi, ctx);
				showStatus(ctx, getActiveConfig());
				break;
			}
			case "context": {
				const selection = await ctx.ui.select("Explain context", ["Full context", "Last N messages"]);
				if (!selection) return;
				if (selection.startsWith("Full")) {
					applyConfig({ contextMessages: -1 }, scope, pi, ctx);
					showStatus(ctx, getActiveConfig());
					break;
				}

				const fallbackValue = scopedConfig.contextMessages > 0 ? String(scopedConfig.contextMessages) : "1";
				const input = await ctx.ui.input("Last N messages (1 or more)", fallbackValue);
				if (!input) return;
				const value = Number(input.trim());
				if (!Number.isFinite(value) || value < 1) {
					notify(ctx, "Invalid value. Use a number greater than 0.");
					continue;
				}
				applyConfig({ contextMessages: Math.floor(value) }, scope, pi, ctx);
				showStatus(ctx, getActiveConfig());
				break;
			}
			case "model": {
				const selectionModel = await chooseModel(ctx, scopedConfig.model, "Preflight model");
				if (!selectionModel) return;
				applyConfig({ model: selectionModel }, scope, pi, ctx);
				showStatus(ctx, getActiveConfig());
				break;
			}
			case "policy-model": {
				const selectionModel = await chooseModel(ctx, scopedConfig.policyModel, "Policy model");
				if (!selectionModel) return;
				applyConfig({ policyModel: selectionModel }, scope, pi, ctx);
				showStatus(ctx, getActiveConfig());
				break;
			}
			case "debug": {
				applyConfig({ debug: !scopedConfig.debug }, scope, pi, ctx);
				showStatus(ctx, getActiveConfig());
				break;
			}
			case "back":
				return;
			default:
				return;
		}
	}
}

function parseScope(args: string[]): ConfigScope {
	const lower = args.map((part) => part.toLowerCase());
	if (lower.includes("--persistent") || lower.includes("persistent") || lower.includes("--persist")) {
		return "persistent";
	}
	return "session";
}

async function chooseModel(
	ctx: ExtensionContext,
	currentModel: PreflightConfig["model"],
	title: string,
): Promise<PreflightConfig["model"] | undefined> {
	if (!ctx.hasUI) return currentModel;

	const mode = await ctx.ui.select(title, [
		`Use current model (${formatModelSetting("current", ctx.model)})`,
		"Pick from available models",
		"Enter provider/model",
	]);
	if (!mode) return undefined;

	if (mode.startsWith("Use current")) {
		return "current";
	}

	if (mode.startsWith("Pick")) {
		const models = ctx.modelRegistry.getAvailable();
		if (models.length === 0) {
			notify(ctx, "No available models with API keys configured.");
			return undefined;
		}
		const labels = models.map((model) => `${model.provider}/${model.id}`);
		const selected = await ctx.ui.select("Select model", labels);
		if (!selected) return undefined;
		const [provider, id] = selected.split("/");
		if (!provider || !id) return undefined;
		return { provider, id };
	}

	const input = await ctx.ui.input("Enter provider/model", formatModelSetting(currentModel, ctx.model));
	if (!input) return undefined;
	return parseModelRef([input]);
}

async function chooseApprovalMode(
	ctx: ExtensionContext,
	currentMode: PreflightConfig["approvalMode"],
): Promise<PreflightConfig["approvalMode"] | undefined> {
	if (!ctx.hasUI) return currentMode;

	const options = [
		{
			value: "auto" as const,
			label: currentMode === "auto" ? "Approve for me (current)" : "Approve for me",
		},
		{
			value: "all" as const,
			label: currentMode === "all" ? "All tools (current)" : "All tools",
		},
		{
			value: "destructive" as const,
			label: currentMode === "destructive" ? "Destructive only (current)" : "Destructive only",
		},
		{
			value: "off" as const,
			label: currentMode === "off" ? "Off (current)" : "Off",
		},
	];

	const selection = await ctx.ui.select("Preflight mode", options.map((option) => option.label));
	if (!selection) return undefined;

	return options.find((option) => option.label === selection)?.value;
}

function applyConfig(
	update: Partial<PreflightConfig>,
	scope: ConfigScope,
	pi: ExtensionAPI,
	ctx: ExtensionContext,
): void {
	if (scope === "persistent") {
		persistentConfig = { ...persistentConfig, ...update };
		savePersistentConfig(persistentConfig);
	} else {
		sessionOverride = { ...sessionOverride, ...update };
		pi.appendEntry(SESSION_ENTRY_TYPE, { config: sessionOverride });
	}
	notify(ctx, "Preflight settings updated.");
}

function clearSessionOverrides(pi: ExtensionAPI): void {
	sessionOverride = {};
	pi.appendEntry(SESSION_ENTRY_TYPE, { config: null });
}

function sessionOverrideExists(): boolean {
	return Object.keys(sessionOverride).length > 0;
}

function showStatus(ctx: ExtensionContext, config: PreflightConfig): void {
	const lines = [
		`Mode: ${formatApprovalMode(config.approvalMode)}`,
		`Explain context: ${formatContextMessages(config.contextMessages)}`,
		`Model: ${formatModelSetting(config.model, ctx.model)}`,
		`Reasoning: ${config.reasoning}`,
		`Policy model: ${formatModelSetting(config.policyModel, ctx.model)}`,
		`Debug: ${config.debug ? "on" : "off"}`,
		`Scope: ${sessionOverrideExists() ? "session override" : "persistent"}`,
	];

	notify(ctx, lines.join("\n"));
}

function buildToolCallsContext(
	toolCalls: ToolCallSummary[],
	ctx: ExtensionContext,
	logDebug: DebugLogger,
): ToolCallsContext {
	return {
		toolCalls,
		llmContext: {
			systemPrompt: ctx.getSystemPrompt(),
			messages: resolveContextMessages(ctx, logDebug),
		},
	};
}

function resolveContextMessages(ctx: ExtensionContext, logDebug: DebugLogger): Message[] {
	if (lastContextSnapshot && lastContextSnapshot.length > 0) {
		logDebug("Using context snapshot for preflight.");
		return convertToLlm(lastContextSnapshot);
	}

	const branchMessages = ctx.sessionManager
		.getBranch()
		.flatMap((entry) => (entry.type === "message" ? [entry.message] : []));

	if (branchMessages.length === 0) {
		logDebug("No context snapshot available for preflight.");
		return [];
	}

	logDebug("Using session branch fallback for preflight context.");
	return convertToLlm(branchMessages);
}

function toToolCallSummary(event: ToolCallEvent): ToolCallSummary {
	return {
		id: event.toolCallId,
		name: event.toolName,
		args: event.input as Record<string, unknown>,
	};
}

export function resolveSiblingToolCalls(
	event: ToolCallEvent,
	ctx: ExtensionContext,
	logDebug: DebugLogger,
): ToolCallSummary[] {
	const branchMessages = ctx.sessionManager
		.getBranch()
		.flatMap((entry) => (entry.type === "message" ? [entry.message] : []));
	for (let index = branchMessages.length - 1; index >= 0; index -= 1) {
		const message = branchMessages[index];
		if (message?.role !== "assistant") continue;
		const siblingCalls = message.content
			.filter(
				(block): block is Extract<(typeof message.content)[number], { type: "toolCall" }> =>
					block.type === "toolCall",
			)
			.map((block) => ({
				id: block.id,
				name: block.name,
				args: block.arguments,
			}));
		if (!siblingCalls.some((call) => call.id === event.toolCallId)) continue;
		const exactCurrent = toToolCallSummary(event);
		logDebug(`Found ${siblingCalls.length} sibling tool call(s) in the current assistant message.`);
		return siblingCalls.map((call) => (call.id === exactCurrent.id ? exactCurrent : call));
	}
	return [toToolCallSummary(event)];
}

export function matchesReviewedInput(
	toolCall: ToolCallSummary,
	cwd: string,
	inputFingerprint: string,
): boolean {
	return buildActionFingerprint(toolCall, cwd).fingerprint === inputFingerprint;
}

export function findConcurrentFileRace(
	event: ToolCallEvent,
	ctx: ExtensionContext,
	logDebug: DebugLogger,
): string | undefined {
	if (!["read", "write", "edit"].includes(event.toolName)) return undefined;
	const siblings = resolveSiblingToolCalls(event, ctx, logDebug);
	const siblingShell = siblings.find(
		(call) => call.id !== event.toolCallId && call.name === "bash",
	);
	if (!siblingShell) return undefined;
	return "Blocked a built-in file call that ran beside a shell call; run these actions in order";
}

async function finalizeAllowedToolCall(
	pi: ExtensionAPI,
	event: ToolCallEvent,
	ctx: ExtensionContext,
	config: PreflightConfig,
	action: ReturnType<typeof buildActionFingerprint>,
	gate: ReturnType<typeof gateToolCall>,
	logDebug: DebugLogger,
): Promise<{ block: true; reason: string } | undefined> {
	const repeated = await maybePersistRepeatedApproval(pi, event, ctx, config, action, logDebug);
	if (repeated) return repeated;
	grantSandboxAccess(pi, event, gate);
	return undefined;
}

function grantSandboxAccess(
	pi: ExtensionAPI,
	event: ToolCallEvent,
	gate: ReturnType<typeof gateToolCall>,
): void {
	if (gate.action !== "review" || !gate.sandboxAllowWrite?.length) return;
	pi.events.emit("guardian:sandbox-allow-once", {
		version: 1,
		toolCallId: event.toolCallId,
		allowWrite: gate.sandboxAllowWrite,
	});
}

async function maybePersistRepeatedApproval(
	pi: ExtensionAPI,
	event: ToolCallEvent,
	ctx: ExtensionContext,
	config: PreflightConfig,
	action: ReturnType<typeof buildActionFingerprint>,
	logDebug: DebugLogger,
): Promise<{ block: true; reason: string } | undefined> {
	if (config.approvalMode !== "auto") return undefined;
	const count = repeatTracker.recordAllowed(action.fingerprint);
	if (count < config.repeatThreshold || !ctx.hasUI) return undefined;

	const approvalId = randomUUID();
	const kind =
		event.toolName === "bash"
			? "command"
			: event.toolName === "mcp_enable"
				? "mcp"
				: "file-write";
	pi.events.emit("approval:requested", {
		version: 1,
		approvalId,
		timestamp: new Date().toISOString(),
		sessionId: ctx.sessionManager.getSessionId(),
		cwd: ctx.cwd,
		kind,
		toolName: event.toolName,
		toolCallId: event.toolCallId,
		ruleId: "guardian:repeated-auto-allow",
		title: "Repeated approved action",
		summary: action.label,
	});

	const selection = await ctx.ui.select(
		"Guardian approved this action more than once",
		[
			"Always allow this exact action in this project",
			"Keep auto-reviewing",
			"Block this action",
		],
	);
	repeatTracker.reset(action.fingerprint);

	if (selection === "Always allow this exact action in this project") {
		try {
			persistProjectGuardianRule(ctx.cwd, action, logDebug, ctx.isProjectTrusted());
			ctx.ui.notify(`Saved project rule: ${action.label}`, "info");
		} catch (error) {
			ctx.ui.notify(error instanceof Error ? error.message : String(error), "error");
		}
		emitRepeatedResolution(pi, event, ctx, approvalId, kind, "allowed");
		return undefined;
	}
	if (selection === "Keep auto-reviewing") {
		emitRepeatedResolution(pi, event, ctx, approvalId, kind, "allowed");
		return undefined;
	}

	const reason = "Blocked by user after repeated automatic approval.";
	emitRepeatedResolution(pi, event, ctx, approvalId, kind, "denied");
	return { block: true, reason };
}

function emitRepeatedResolution(
	pi: ExtensionAPI,
	event: ToolCallEvent,
	ctx: ExtensionContext,
	approvalId: string,
	kind: string,
	decision: "allowed" | "denied",
): void {
	pi.events.emit("approval:resolved", {
		version: 1,
		approvalId,
		timestamp: new Date().toISOString(),
		sessionId: ctx.sessionManager.getSessionId(),
		cwd: ctx.cwd,
		kind,
		toolName: event.toolName,
		toolCallId: event.toolCallId,
		ruleId: "guardian:repeated-auto-allow",
		decision,
	});
}
