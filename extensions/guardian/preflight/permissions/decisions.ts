import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import type {
	DebugLogger,
	PermissionRule,
	PermissionsState,
	PolicyOverrideRule,
	PreflightConfig,
	ToolPreflightMetadata,
	ToolCallSummary,
	ToolCallsContext,
	ToolDecision,
	ToolPolicyDecision,
} from "../types.js";
import {
	formatRuleLabel,
	getPolicyRulesForTool,
	matchesPermissionRule,
	matchesPolicyOverride,
} from "./matching.js";

export async function resolveToolDecisions(
	event: ToolCallsContext,
	preflight: Record<string, ToolPreflightMetadata>,
	policyDecisions: Record<string, ToolPolicyDecision>,
	ctx: ExtensionContext,
	config: PreflightConfig,
	permissions: PermissionsState,
	logDebug: DebugLogger,
): Promise<Record<string, ToolDecision>> {
	const decisions: Record<string, ToolDecision> = {};

	for (const toolCall of event.toolCalls) {
		const metadata = preflight[toolCall.id];
		const deterministic = resolveDeterministicDecisionForToolCall(toolCall, ctx.cwd, permissions, logDebug);
		if (deterministic && !(deterministic.decision === "ask" && config.approvalMode === "auto")) {
			const decision: ToolDecision = {
				decision: deterministic.decision,
				source: "deterministic",
				reason: deterministic.reason,
				rule: deterministic.rule,
			};
			decisions[toolCall.id] = decision;
			logFinalDecision(toolCall, decision, logDebug);
			continue;
		}

		const overrideRule = findMatchingPolicyOverride(toolCall, permissions.policyOverrides, ctx.cwd);
		if (overrideRule) {
			logDebug(`Policy override matched: ${formatRuleLabel(overrideRule)}.`);
		}

		const applicablePolicyRules = getPolicyRulesForTool(toolCall.name, permissions.policyRules);
		const policyDecision = policyDecisions[toolCall.id];
		if (!overrideRule && applicablePolicyRules.length > 0 && policyDecision && policyDecision.decision !== "none") {
			const shouldPromptForPolicyDeny =
				policyDecision.decision === "deny" &&
				config.approvalMode !== "off" &&
				config.approvalMode !== "auto" &&
				ctx.hasUI;
			const decision: ToolDecision = {
				decision: shouldPromptForPolicyDeny ? "ask" : policyDecision.decision,
				source: "policy",
				reason:
					policyDecision.decision === "deny" && !shouldPromptForPolicyDeny
						? buildPolicyDenyReason(policyDecision.reason)
						: undefined,
				policy: {
					decision: policyDecision.decision,
					reason: policyDecision.reason,
					rules: applicablePolicyRules,
				},
			};
			decisions[toolCall.id] = decision;
			logFinalDecision(toolCall, decision, logDebug);
			continue;
		}

		const fallbackDecision = buildDefaultDecision(metadata, config.approvalMode);
		const decision: ToolDecision = {
			decision: fallbackDecision,
			source: "fallback",
			reason: fallbackDecision === "deny" ? metadata?.rationale : undefined,
		};
		decisions[toolCall.id] = decision;
		logFinalDecision(toolCall, decision, logDebug);
	}

	return decisions;
}

export function resolveDeterministicDecisionForToolCall(
	toolCall: ToolCallSummary,
	cwd: string,
	permissions: PermissionsState,
	logDebug: DebugLogger,
): { decision: "allow" | "ask" | "deny"; rule?: PermissionRule; reason?: string } | undefined {
	const denyRule = findMatchingRule(toolCall, permissions.rules.deny, cwd);
	if (denyRule) {
		logDebug(`Permission rule matched (deny): ${formatRuleLabel(denyRule)}.`);
		return {
			decision: "deny",
			rule: denyRule,
			reason: buildPermissionDenyReason(denyRule),
		};
	}

	const askRule = findMatchingRule(toolCall, permissions.rules.ask, cwd);
	if (askRule) {
		logDebug(`Permission rule matched (ask): ${formatRuleLabel(askRule)}.`);
		return { decision: "ask", rule: askRule };
	}

	const allowRule = findMatchingRule(toolCall, permissions.rules.allow, cwd);
	if (allowRule) {
		logDebug(`Permission rule matched (allow): ${formatRuleLabel(allowRule)}.`);
		return { decision: "allow", rule: allowRule };
	}

	return undefined;
}

function buildDefaultDecision(
	metadata: ToolPreflightMetadata | undefined,
	mode: PreflightConfig["approvalMode"],
): "allow" | "ask" | "deny" {
	if (mode === "off") return "allow";
	if (mode === "auto") return metadata?.outcome ?? "deny";
	if (mode === "all") return "ask";
	if (mode === "destructive") {
		const destructive = metadata?.destructive ?? true;
		return destructive ? "ask" : "allow";
	}
	return "ask";
}

function buildPermissionDenyReason(rule: { raw: string }): string {
	return `Blocked by rule ${rule.raw}.`;
}

function buildPolicyDenyReason(reason: string): string {
	return `Blocked by custom rules: ${reason}`;
}

function findMatchingRule(
	toolCall: ToolCallSummary,
	rules: PermissionRule[],
	cwd: string,
): PermissionRule | undefined {
	for (const rule of rules) {
		if (!rule) continue;
		if (matchesPermissionRule(rule, toolCall, cwd)) {
			return rule;
		}
	}
	return undefined;
}

function findMatchingPolicyOverride(
	toolCall: ToolCallSummary,
	rules: PolicyOverrideRule[],
	cwd: string,
): PolicyOverrideRule | undefined {
	for (const rule of rules) {
		if (matchesPolicyOverride(rule, toolCall, cwd)) {
			return rule;
		}
	}
	return undefined;
}

function logFinalDecision(toolCall: ToolCallSummary, decision: ToolDecision, logDebug: DebugLogger): void {
	const reasonPart = decision.reason ? `, reason: ${decision.reason}` : "";
	logDebug(
		`Final decision for ${toolCall.id} (${toolCall.name}): ${decision.decision} (source: ${decision.source}${reasonPart}).`,
	);
}
