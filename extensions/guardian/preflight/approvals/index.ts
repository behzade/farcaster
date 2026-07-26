import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import type {
	DebugLogger,
	PreflightConfig,
	ToolPreflightMetadata,
	ToolCallSummary,
	ToolCallsContext,
	ToolDecision,
} from "../types.js";
import { buildPreflightMetadata } from "../preflight.js";
import { resolveToolDecisions } from "../permissions/decisions.js";
import { getPolicyRulesForTool } from "../permissions/matching.js";
import { persistPolicyOverride, persistPolicyRule, persistWorkspaceRule } from "../permissions/persistence.js";
import { loadPermissionsState } from "../permissions/state.js";
import { buildRuleContextSnapshot } from "../rule-context.js";
import { evaluateRuleConsistency } from "../rule-consistency.js";
import { requestApproval, requestRuleConflictAction } from "./approval-ui.js";

export async function collectApprovals(
	event: ToolCallsContext,
	preflight: Record<string, ToolPreflightMetadata>,
	decisions: Record<string, ToolDecision>,
	ctx: ExtensionContext,
	config: PreflightConfig,
	logDebug: DebugLogger,
): Promise<Record<string, { allow: boolean; reason?: string }> | undefined> {
	const approvals: Record<string, { allow: boolean; reason?: string }> = {};
	let approvalTargets = 0;

	for (const toolCall of event.toolCalls) {
		const decision = decisions[toolCall.id];
		if (!decision) continue;

		if (decision.decision === "deny") {
			approvals[toolCall.id] = {
				allow: false,
				reason: decision.reason ?? "Blocked by policy",
			};
			continue;
		}

		if (decision.decision === "allow") {
			continue;
		}

		if (!ctx.hasUI) {
			approvals[toolCall.id] = {
				allow: false,
				reason: "Approval required but no UI available.",
			};
			continue;
		}

		approvalTargets += 1;
	}

	if (approvalTargets === 0) {
		if (Object.keys(approvals).length === 0) {
			logDebug("No tool calls require approval.");
			return undefined;
		}
		return approvals;
	}

	logDebug(`Requesting approvals for ${approvalTargets} tool call(s).`);

	for (const toolCall of event.toolCalls) {
		const initialDecision = decisions[toolCall.id];
		if (!initialDecision || initialDecision.decision !== "ask") continue;
		if (!ctx.hasUI) continue;

		let currentDecision = initialDecision;
		let currentMetadata = preflight[toolCall.id];

		while (true) {
			const permissions = loadPermissionsState(ctx.cwd, logDebug, ctx.isProjectTrusted());
			const existingRules = buildRuleContextSnapshot(toolCall.name, permissions);
			const approvalDecision = await requestApproval(
				event,
				toolCall,
				currentMetadata,
				currentDecision,
				existingRules,
				ctx,
				config,
				logDebug,
			);
			const approvalDetail =
				approvalDecision.action === "custom-rule"
					? ` (${approvalDecision.rule})`
					: "";
			logDebug(`Approval decision for ${toolCall.name}: ${approvalDecision.action}${approvalDetail}.`);

			if (approvalDecision.action === "allow-persist") {
				persistWorkspaceRule(toolCall, "allow", ctx, logDebug);
				if (currentDecision.policy?.decision === "deny") {
					persistPolicyOverride(toolCall, ctx, logDebug);
				}
				approvals[toolCall.id] = { allow: true };
				break;
			}

			if (approvalDecision.action === "allow") {
				approvals[toolCall.id] = { allow: true };
				break;
			}

			if (approvalDecision.action === "deny") {
				approvals[toolCall.id] = { allow: false, reason: "Blocked by user" };
				break;
			}

			const consistency = await evaluateRuleConsistency(
				event,
				toolCall,
				approvalDecision.rule,
				existingRules,
				ctx,
				config,
				logDebug,
			);
			logDebug(
				`Rule consistency for ${toolCall.name}: conflict=${consistency.conflict}, reason=${consistency.reason}`,
			);
			if (consistency.conflict) {
				const conflictAction = await requestRuleConflictAction(
					toolCall,
					approvalDecision.rule,
					consistency,
					ctx,
				);
				logDebug(`Rule conflict action for ${toolCall.name}: ${conflictAction}.`);
				if (conflictAction === "edit-rule") {
					continue;
				}
				if (conflictAction === "cancel") {
					approvals[toolCall.id] = { allow: false, reason: "Blocked by user" };
					break;
				}
			}

			persistPolicyRule(toolCall, approvalDecision.rule, ctx, logDebug);
			const applied = await applyCustomRuleToCurrentCall(
				event,
				toolCall,
				ctx,
				config,
				logDebug,
			);

			if (applied.status === "error") {
				approvals[toolCall.id] = { allow: false, reason: applied.reason };
				break;
			}

			currentDecision = applied.decision;
			currentMetadata = applied.metadata;
			logDebug(
				`Custom rule applied for ${toolCall.name}. Immediate decision: ${currentDecision.decision} (${currentDecision.source}).`,
			);

			if (currentDecision.decision === "allow") {
				approvals[toolCall.id] = { allow: true };
				break;
			}

			if (currentDecision.decision === "deny") {
				approvals[toolCall.id] = {
					allow: false,
					reason: currentDecision.reason ?? "Blocked by custom rule",
				};
				break;
			}

			// ask: open approval prompt again with updated policy output.
		}
	}

	return Object.keys(approvals).length > 0 ? approvals : undefined;
}

async function applyCustomRuleToCurrentCall(
	event: ToolCallsContext,
	toolCall: ToolCallSummary,
	ctx: ExtensionContext,
	config: PreflightConfig,
	logDebug: DebugLogger,
): Promise<
	| { status: "ok"; metadata: ToolPreflightMetadata; decision: ToolDecision }
	| { status: "error"; reason: string }
> {
	const permissions = loadPermissionsState(ctx.cwd, logDebug, ctx.isProjectTrusted());
	const scopedEvent: ToolCallsContext = {
		toolCalls: [toolCall],
		llmContext: event.llmContext,
	};
	const policyRulesByToolCall = {
		[toolCall.id]: getPolicyRulesForTool(toolCall.name, permissions.policyRules),
	};

	const preflightResult = await buildPreflightMetadata(
		scopedEvent,
		policyRulesByToolCall,
		ctx,
		config,
		logDebug,
	);
	if (preflightResult.status !== "ok") {
		return {
			status: "error",
			reason: `Failed to validate custom rule: ${preflightResult.reason}`,
		};
	}

	const nextDecisions = await resolveToolDecisions(
		scopedEvent,
		preflightResult.metadata,
		preflightResult.policyDecisions,
		ctx,
		config,
		permissions,
		logDebug,
	);
	const nextDecision = nextDecisions[toolCall.id];
	if (!nextDecision) {
		return {
			status: "error",
			reason: "Failed to evaluate custom rule for current tool call.",
		};
	}

	return {
		status: "ok",
		metadata: preflightResult.metadata[toolCall.id],
		decision: nextDecision,
	};
}

export function buildAllowAllApprovals(
	toolCalls: ToolCallSummary[],
): Record<string, { allow: boolean; reason?: string }> {
	const approvals: Record<string, { allow: boolean; reason?: string }> = {};
	for (const toolCall of toolCalls) {
		approvals[toolCall.id] = { allow: true };
	}
	return approvals;
}

export function buildBlockAllApprovals(
	toolCalls: ToolCallSummary[],
	reason: string,
): Record<string, { allow: boolean; reason?: string }> {
	const approvals: Record<string, { allow: boolean; reason?: string }> = {};
	for (const toolCall of toolCalls) {
		approvals[toolCall.id] = { allow: false, reason };
	}
	return approvals;
}
