import {
	getPermissionRulePatternsForTool,
	getPolicyOverridesForTool,
	getPolicyRuleBucketsForTool,
	normalizeToolName,
} from "./permissions/matching.js";
import type { PermissionsState, RuleContextSnapshot } from "./types.js";

export function buildRuleContextSnapshot(
	toolName: string,
	permissions: PermissionsState,
): RuleContextSnapshot {
	const tool = normalizeToolName(toolName);
	const policy = getPolicyRuleBucketsForTool(tool, permissions.policyRules);

	return {
		tool,
		policy,
		permissions: {
			allow: getPermissionRulePatternsForTool(tool, permissions.rules.allow),
			ask: getPermissionRulePatternsForTool(tool, permissions.rules.ask),
			deny: getPermissionRulePatternsForTool(tool, permissions.rules.deny),
		},
		policyOverrides: getPolicyOverridesForTool(tool, permissions.policyOverrides),
	};
}

export function getPolicyRuleCandidates(snapshot: RuleContextSnapshot): string[] {
	const seen = new Set<string>();
	const candidates: string[] = [];
	for (const rule of [...snapshot.policy.global, ...snapshot.policy.tool]) {
		if (seen.has(rule)) continue;
		seen.add(rule);
		candidates.push(rule);
	}
	return candidates;
}
