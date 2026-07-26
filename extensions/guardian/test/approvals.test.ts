import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import type { Context } from "@earendil-works/pi-ai";
import type { PreflightConfig, ToolCallsContext, ToolDecision, ToolPreflightMetadata } from "../preflight/types.js";
import { collectApprovals } from "../preflight/approvals/index.js";
import {
	requestApproval,
	requestRuleConflictAction,
} from "../preflight/approvals/approval-ui.js";
import { persistPolicyRule } from "../preflight/permissions/persistence.js";
import { buildPreflightMetadata } from "../preflight/preflight.js";
import { evaluateRuleConsistency } from "../preflight/rule-consistency.js";
import { resolveToolDecisions } from "../preflight/permissions/decisions.js";

vi.mock("../preflight/approvals/approval-ui.js", () => ({
	requestApproval: vi.fn(),
	requestRuleConflictAction: vi.fn(),
}));

vi.mock("../preflight/permissions/persistence.js", () => ({
	persistWorkspaceRule: vi.fn(),
	persistPolicyOverride: vi.fn(),
	persistPolicyRule: vi.fn(),
}));

vi.mock("../preflight/permissions/state.js", () => ({
	loadPermissionsState: vi.fn(() => ({
		rules: { allow: [], ask: [], deny: [] },
		policyRules: [],
		policyOverrides: [],
	})),
}));

vi.mock("../preflight/permissions/matching.js", async () => {
	const actual = await vi.importActual<typeof import("../preflight/permissions/matching.js")>(
		"../preflight/permissions/matching.js",
	);
	return {
		...actual,
		getPolicyRulesForTool: vi.fn(() => ["test rule"]),
	};
});

vi.mock("../preflight/preflight.js", () => ({
	buildPreflightMetadata: vi.fn(),
}));

vi.mock("../preflight/permissions/decisions.js", () => ({
	resolveToolDecisions: vi.fn(),
}));

vi.mock("../preflight/rule-consistency.js", () => ({
	evaluateRuleConsistency: vi.fn(),
}));

const logDebug = () => {};

const config: PreflightConfig = {
	contextMessages: 1,
	explainKey: "ctrl+e",
	ruleSuggestionKey: "ctrl+n",
	model: "current",
	policyModel: "current",
	reasoning: "low",
	approvalMode: "all",
	repeatThreshold: 2,
	debug: false,
};

const llmContext: Context = { messages: [] };

function buildEvent(): ToolCallsContext {
	return {
		toolCalls: [{ id: "call-1", name: "bash", args: { command: "ls -la" } }],
		llmContext,
	};
}

function buildAskDecision(): Record<string, ToolDecision> {
	return {
		"call-1": {
			decision: "ask",
			source: "fallback",
		},
	};
}

function buildMetadata(): Record<string, ToolPreflightMetadata> {
	return {
		"call-1": {
			summary: "Run command",
			destructive: false,
			riskLevel: "low",
			userAuthorization: "high",
			outcome: "allow",
			rationale: "Narrow action.",
		},
	};
}

function createCtx(): ExtensionContext {
	return {
		cwd: "/workspace",
		hasUI: true,
		ui: {},
		isProjectTrusted: () => true,
	} as ExtensionContext;
}

describe("collectApprovals custom rule behavior", () => {
	const approvalMock = vi.mocked(requestApproval);
	const conflictActionMock = vi.mocked(requestRuleConflictAction);
	const consistencyMock = vi.mocked(evaluateRuleConsistency);
	const persistPolicyRuleMock = vi.mocked(persistPolicyRule);
	const preflightMock = vi.mocked(buildPreflightMetadata);
	const decisionsMock = vi.mocked(resolveToolDecisions);

	beforeEach(() => {
		approvalMock.mockReset();
		conflictActionMock.mockReset();
		consistencyMock.mockReset();
		persistPolicyRuleMock.mockReset();
		preflightMock.mockReset();
		decisionsMock.mockReset();
		consistencyMock.mockResolvedValue({
			conflict: false,
			reason: "No conflict detected.",
			conflictsWith: [],
		});
	});

	it("does not auto-allow when custom rule evaluates to deny", async () => {
		approvalMock.mockResolvedValue({ action: "custom-rule", rule: "Never allow shell commands" });
		preflightMock.mockResolvedValue({
			status: "ok",
			metadata: buildMetadata(),
			policyDecisions: {
				"call-1": { decision: "deny", reason: "Rule denies this command" },
			},
		});
		decisionsMock.mockResolvedValue({
			"call-1": {
				decision: "deny",
				source: "policy",
				reason: "Blocked by custom rules: Rule denies this command",
				policy: {
					decision: "deny",
					reason: "Rule denies this command",
					rules: ["Never allow shell commands"],
				},
			},
		});

		const approvals = await collectApprovals(
			buildEvent(),
			buildMetadata(),
			buildAskDecision(),
			createCtx(),
			config,
			logDebug,
		);

		expect(persistPolicyRuleMock).toHaveBeenCalledOnce();
		expect(approvals?.["call-1"]).toEqual({
			allow: false,
			reason: "Blocked by custom rules: Rule denies this command",
		});
	});

	it("loops back to editor when conflict action is edit rule", async () => {
		approvalMock
			.mockResolvedValueOnce({ action: "custom-rule", rule: "Allow list commands" })
			.mockResolvedValueOnce({ action: "allow" });
		consistencyMock.mockResolvedValue({
			conflict: true,
			reason: "Conflicts with existing deny rule.",
			conflictsWith: ["Deny all shell commands"],
		});
		conflictActionMock.mockResolvedValue("edit-rule");

		const approvals = await collectApprovals(
			buildEvent(),
			buildMetadata(),
			buildAskDecision(),
			createCtx(),
			config,
			logDebug,
		);

		expect(approvalMock).toHaveBeenCalledTimes(2);
		expect(persistPolicyRuleMock).not.toHaveBeenCalled();
		expect(approvals?.["call-1"]).toEqual({ allow: true });
	});

	it("saves conflicting rule when user chooses save anyway", async () => {
		approvalMock.mockResolvedValue({ action: "custom-rule", rule: "Allow list commands" });
		consistencyMock.mockResolvedValue({
			conflict: true,
			reason: "Conflicts with existing ask rule.",
			conflictsWith: ["Ask before any shell command"],
		});
		conflictActionMock.mockResolvedValue("save-anyway");
		preflightMock.mockResolvedValue({
			status: "ok",
			metadata: buildMetadata(),
			policyDecisions: {
				"call-1": { decision: "allow", reason: "Rule allows list commands" },
			},
		});
		decisionsMock.mockResolvedValue({
			"call-1": {
				decision: "allow",
				source: "policy",
				policy: {
					decision: "allow",
					reason: "Rule allows list commands",
					rules: ["Allow list commands"],
				},
			},
		});

		const approvals = await collectApprovals(
			buildEvent(),
			buildMetadata(),
			buildAskDecision(),
			createCtx(),
			config,
			logDebug,
		);

		expect(persistPolicyRuleMock).toHaveBeenCalledOnce();
		expect(approvals?.["call-1"]).toEqual({ allow: true });
	});

	it("cancels approval when conflicting rule is canceled", async () => {
		approvalMock.mockResolvedValue({ action: "custom-rule", rule: "Allow list commands" });
		consistencyMock.mockResolvedValue({
			conflict: true,
			reason: "Conflicts with existing deny rule.",
			conflictsWith: ["Deny shell writes"],
		});
		conflictActionMock.mockResolvedValue("cancel");

		const approvals = await collectApprovals(
			buildEvent(),
			buildMetadata(),
			buildAskDecision(),
			createCtx(),
			config,
			logDebug,
		);

		expect(persistPolicyRuleMock).not.toHaveBeenCalled();
		expect(preflightMock).not.toHaveBeenCalled();
		expect(approvals?.["call-1"]).toEqual({ allow: false, reason: "Blocked by user" });
	});

	it("allows current call when custom rule evaluates to allow", async () => {
		approvalMock.mockResolvedValue({ action: "custom-rule", rule: "Allow list commands" });
		preflightMock.mockResolvedValue({
			status: "ok",
			metadata: buildMetadata(),
			policyDecisions: {
				"call-1": { decision: "allow", reason: "Rule allows list commands" },
			},
		});
		decisionsMock.mockResolvedValue({
			"call-1": {
				decision: "allow",
				source: "policy",
				policy: {
					decision: "allow",
					reason: "Rule allows list commands",
					rules: ["Allow list commands"],
				},
			},
		});

		const approvals = await collectApprovals(
			buildEvent(),
			buildMetadata(),
			buildAskDecision(),
			createCtx(),
			config,
			logDebug,
		);

		expect(approvals?.["call-1"]).toEqual({ allow: true });
	});

	it("re-opens approval when custom rule evaluates to ask", async () => {
		approvalMock
			.mockResolvedValueOnce({ action: "custom-rule", rule: "Ask before shell commands" })
			.mockResolvedValueOnce({ action: "allow" });
		preflightMock.mockResolvedValue({
			status: "ok",
			metadata: buildMetadata(),
			policyDecisions: {
				"call-1": { decision: "ask", reason: "Policy still requires confirmation" },
			},
		});
		decisionsMock.mockResolvedValue({
			"call-1": {
				decision: "ask",
				source: "policy",
				policy: {
					decision: "ask",
					reason: "Policy still requires confirmation",
					rules: ["Ask before shell commands"],
				},
			},
		});

		const approvals = await collectApprovals(
			buildEvent(),
			buildMetadata(),
			buildAskDecision(),
			createCtx(),
			config,
			logDebug,
		);

		expect(approvalMock).toHaveBeenCalledTimes(2);
		expect(approvals?.["call-1"]).toEqual({ allow: true });
	});
});
