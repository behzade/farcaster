import type { Api, Context, Model, ProviderEnv } from "@earendil-works/pi-ai";
import type { KeyId } from "@earendil-works/pi-tui";

export interface ToolPreflightMetadata {
	summary: string;
	destructive: boolean;
	scope?: string[];
	riskLevel: "low" | "medium" | "high" | "critical";
	userAuthorization: "high" | "medium" | "low" | "unknown";
	outcome: "allow" | "deny";
	rationale: string;
}

export interface ToolCallSummary {
	id: string;
	name: string;
	args: Record<string, unknown>;
}

export interface ToolCallsContext {
	toolCalls: ToolCallSummary[];
	llmContext: Context;
}

export type DebugLogger = (message: string) => void;

export type ConfigScope = "session" | "persistent";

export type ModelRef = { provider: string; id: string };

export type ApprovalMode = "auto" | "all" | "destructive" | "off";

export interface PreflightConfig {
	contextMessages: number;
	explainKey: KeyId | KeyId[];
	ruleSuggestionKey: KeyId | KeyId[];
	model: "current" | ModelRef;
	policyModel: "current" | ModelRef;
	approvalMode: ApprovalMode;
	repeatThreshold: number;
	debug: boolean;
}

export interface SessionConfigEntryData {
	config?: unknown;
}

export type PermissionDecision = "allow" | "ask" | "deny";

export type PermissionSource = "global" | "workspace";

export interface PermissionSettingsFile {
	version?: number;
	permissions?: Record<string, unknown>;
	preflight?: Record<string, unknown>;
}

export interface PermissionRule {
	kind: PermissionDecision;
	raw: string;
	tool: string;
	specifier?: string;
	source: PermissionSource;
	settingsPath: string;
	settingsDir: string;
	argsMatch?: unknown;
}

export interface PolicyRule {
	tool: string;
	policy: string;
	source: PermissionSource;
	settingsPath: string;
	settingsDir: string;
}

export interface PolicyOverrideRule {
	raw: string;
	tool: string;
	specifier?: string;
	source: PermissionSource;
	settingsPath: string;
	settingsDir: string;
	argsMatch?: unknown;
}

export interface PermissionRules {
	allow: PermissionRule[];
	ask: PermissionRule[];
	deny: PermissionRule[];
}

export interface PermissionsState {
	rules: PermissionRules;
	policyRules: PolicyRule[];
	policyOverrides: PolicyOverrideRule[];
}

export interface RuleContextSnapshot {
	tool: string;
	policy: {
		global: string[];
		tool: string[];
	};
	permissions: {
		allow: string[];
		ask: string[];
		deny: string[];
	};
	policyOverrides: string[];
}

export interface PolicyEvaluation {
	decision: PermissionDecision;
	reason?: string;
	rules: string[];
}

export interface ToolPolicyDecision {
	decision: PermissionDecision | "none";
	reason: string;
}

export interface ToolDecision {
	decision: PermissionDecision;
	source: "deterministic" | "policy" | "fallback";
	reason?: string;
	rule?: PermissionRule;
	policy?: PolicyEvaluation;
}

export type PreflightAttempt =
	| {
			status: "ok";
			metadata: Record<string, ToolPreflightMetadata>;
			policyDecisions: Record<string, ToolPolicyDecision>;
	  }
	| { status: "error"; reason: string };

export type ExplanationAttempt =
	| { status: "ok"; text: string }
	| { status: "error"; reason: string };

export type RuleSuggestionAttempt =
	| { status: "ok"; suggestions: string[] }
	| { status: "error"; reason: string };

export interface RuleConsistencyResult {
	conflict: boolean;
	reason: string;
	conflictsWith: string[];
}

export type RuleConflictAction = "edit-rule" | "save-anyway" | "cancel";

export type PreflightFailureDecision =
	| { action: "retry" }
	| { action: "allow" }
	| { action: "block"; reason: string };

export type ApprovalDecision =
	| { action: "allow" }
	| { action: "allow-persist" }
	| { action: "deny" }
	| { action: "custom-rule"; rule: string };

export type ModelWithKey = {
	model: Model<Api>;
	apiKey: string;
	headers?: Record<string, string>;
	env?: ProviderEnv;
};
