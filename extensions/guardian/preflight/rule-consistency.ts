import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import { streamSimple } from "@earendil-works/pi-ai/compat";
import type { Context } from "@earendil-works/pi-ai";
import {
	createUserMessage,
	extractJsonPayload,
	extractText,
	limitContextMessages,
	resolveModelWithApiKey,
	stripCodeFence,
} from "./llm-utils.js";
import type {
	DebugLogger,
	PreflightConfig,
	RuleConsistencyResult,
	RuleContextSnapshot,
	ToolCallSummary,
	ToolCallsContext,
} from "./types.js";

export async function evaluateRuleConsistency(
	event: ToolCallsContext,
	toolCall: ToolCallSummary,
	candidateRule: string,
	existingRules: RuleContextSnapshot,
	ctx: ExtensionContext,
	config: PreflightConfig,
	logDebug: DebugLogger,
	signal?: AbortSignal,
): Promise<RuleConsistencyResult> {
	const modelWithKey = await resolveModelWithApiKey(ctx, config.policyModel);
	if (!modelWithKey) {
		const reason = "No model or API key available for consistency check.";
		logDebug(`Rule consistency fallback: ${reason}`);
		return buildUnavailableResult(reason);
	}

	const instruction = buildRuleConsistencyPrompt(toolCall, candidateRule, existingRules);
	const trimmedContext = limitContextMessages(event.llmContext.messages, 0);
	const consistencyContext: Context = {
		...event.llmContext,
		messages: [...trimmedContext, createUserMessage(instruction)],
	};

	logDebug(`Rule consistency prompt:\n${instruction}`);

	try {
		const response = await streamSimple(modelWithKey.model, consistencyContext, {
			apiKey: modelWithKey.apiKey,
			headers: modelWithKey.headers,
			env: modelWithKey.env,
			signal,
			reasoning: config.reasoning,
		});
		for await (const _ of response) {
			// Drain stream to completion.
		}
		const result = await response.result();
		const text = extractText(result.content);
		logDebug(`Rule consistency raw response:\n${text ?? ""}`);
		const parsed = parseRuleConsistencyResponse(text);
		if (!parsed) {
			const reason = "Rule consistency response was not valid JSON.";
			logDebug(`Rule consistency fallback: ${reason}`);
			return buildUnavailableResult(reason);
		}
		return parsed;
	} catch (error) {
		if (signal?.aborted) {
			return buildUnavailableResult("Rule consistency request cancelled.");
		}
		const message = error instanceof Error ? error.message : String(error);
		const reason = message
			? `Rule consistency request failed: ${message}`
			: "Rule consistency request failed.";
		logDebug(`Rule consistency fallback: ${reason}`);
		return buildUnavailableResult(reason);
	}
}

function buildRuleConsistencyPrompt(
	toolCall: ToolCallSummary,
	candidateRule: string,
	existingRules: RuleContextSnapshot,
): string {
	const lines = [
		"You are validating whether a new custom policy rule conflicts with existing rules.",
		"Return JSON only with this exact shape:",
		"{ \"conflict\": boolean, \"reason\": string, \"conflictsWith\": string[] }",
		"Set conflict=true when the candidate rule duplicates or contradicts existing policy/deterministic rules.",
		"If uncertain, set conflict=false and explain uncertainty in reason.",
		"No markdown. No extra keys.",
		`Candidate rule: ${candidateRule}`,
		...buildRulesContextSection("Existing policy rules (global)", existingRules.policy.global),
		...buildRulesContextSection("Existing policy rules (tool-specific)", existingRules.policy.tool),
		...buildRulesContextSection("Deterministic permissions (allow)", existingRules.permissions.allow),
		...buildRulesContextSection("Deterministic permissions (ask)", existingRules.permissions.ask),
		...buildRulesContextSection("Deterministic permissions (deny)", existingRules.permissions.deny),
		...buildRulesContextSection("Policy overrides", existingRules.policyOverrides),
		"Tool call:",
		JSON.stringify(toolCall, null, 2),
	];

	return lines.join("\n");
}

function buildRulesContextSection(title: string, rules: string[]): string[] {
	if (rules.length === 0) {
		return [`${title}: (none)`];
	}
	return [`${title}:`, ...rules.map((rule) => `- ${rule}`)];
}

export function parseRuleConsistencyResponse(text: string | undefined): RuleConsistencyResult | undefined {
	if (!text) return undefined;
	const cleaned = stripCodeFence(text.trim());
	const jsonText = extractJsonPayload(cleaned);
	if (!jsonText) return undefined;

	try {
		const parsed = JSON.parse(jsonText) as unknown;
		if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
			return undefined;
		}
		const record = parsed as {
			conflict?: unknown;
			reason?: unknown;
			conflictsWith?: unknown;
		};
		if (typeof record.conflict !== "boolean") {
			return undefined;
		}
		const reason =
			typeof record.reason === "string" && record.reason.trim().length > 0
				? record.reason.trim()
				: record.conflict
					? "Potential conflict detected."
					: "No conflict detected.";
		const conflictsWith = Array.isArray(record.conflictsWith)
			? Array.from(
					new Set(
						record.conflictsWith
							.filter((item): item is string => typeof item === "string")
							.map((item) => item.trim())
							.filter((item) => item.length > 0),
					),
			  )
			: [];
		return {
			conflict: record.conflict,
			reason,
			conflictsWith,
		};
	} catch {
		return undefined;
	}
}

function buildUnavailableResult(reason: string): RuleConsistencyResult {
	return {
		conflict: false,
		reason: `Consistency check unavailable: ${reason}`,
		conflictsWith: [],
	};
}
