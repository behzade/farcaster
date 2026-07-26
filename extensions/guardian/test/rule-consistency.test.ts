import { describe, expect, it, vi } from "vitest";
import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import type { Api, Model } from "@earendil-works/pi-ai";
import { streamSimple } from "@earendil-works/pi-ai/compat";
import {
	evaluateRuleConsistency,
	parseRuleConsistencyResponse,
} from "../preflight/rule-consistency.js";
import type { PreflightConfig, RuleContextSnapshot, ToolCallsContext } from "../preflight/types.js";

vi.mock("@earendil-works/pi-ai/compat", async () => {
	const actual = await vi.importActual<typeof import("@earendil-works/pi-ai/compat")>("@earendil-works/pi-ai/compat");
	return {
		...actual,
		streamSimple: vi.fn(),
	};
});

const model = { provider: "openai-codex", id: "gpt-5.6-luna" } as Model<Api>;

const baseConfig: PreflightConfig = {
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

const event: ToolCallsContext = {
	toolCalls: [{ id: "call-1", name: "bash", args: { command: "ls -la" } }],
	llmContext: { messages: [] },
};

const existingRules: RuleContextSnapshot = {
	tool: "bash",
	policy: { global: ["Ask before destructive commands"], tool: ["Allow list commands"] },
	permissions: { allow: ["Bash(ls:*)"], ask: [], deny: ["Bash(rm -rf*)"] },
	policyOverrides: [],
};

function createContext(withApiKey: boolean): ExtensionContext {
	return {
		cwd: "/workspace",
		hasUI: false,
		model,
		modelRegistry: {
			find: vi.fn(),
			getApiKeyAndHeaders: vi
				.fn()
				.mockResolvedValue(withApiKey ? { ok: true, apiKey: "test-key" } : { ok: false, error: "missing" }),
		},
	} as unknown as ExtensionContext;
}

describe("rule consistency parsing", () => {
	it("parses valid JSON shape", () => {
		const parsed = parseRuleConsistencyResponse(
			'{"conflict":true,"reason":"Conflicts with deny","conflictsWith":["Deny writes"]}',
		);

		expect(parsed).toEqual({
			conflict: true,
			reason: "Conflicts with deny",
			conflictsWith: ["Deny writes"],
		});
	});

	it("returns undefined for invalid JSON shape", () => {
		expect(parseRuleConsistencyResponse("not json")).toBeUndefined();
		expect(parseRuleConsistencyResponse('{"conflictsWith":[]}')).toBeUndefined();
	});
});

describe("evaluateRuleConsistency", () => {
	it("returns non-blocking fallback when model is unavailable", async () => {
		const result = await evaluateRuleConsistency(
			event,
			event.toolCalls[0],
			"Allow list commands",
			existingRules,
			createContext(false),
			baseConfig,
			() => {},
		);

		expect(result.conflict).toBe(false);
		expect(result.reason).toContain("Consistency check unavailable");
	});

	it("returns non-blocking fallback when response is invalid", async () => {
		const streamMock = vi.mocked(streamSimple);
		streamMock.mockResolvedValue({
			async *[Symbol.asyncIterator]() {
				return;
			},
			result: async () => ({
				content: [{ type: "text", text: "invalid response" }],
			}),
		} as unknown as Awaited<ReturnType<typeof streamSimple>>);

		const result = await evaluateRuleConsistency(
			event,
			event.toolCalls[0],
			"Allow list commands",
			existingRules,
			createContext(true),
			baseConfig,
			() => {},
		);

		expect(result).toEqual({
			conflict: false,
			reason: "Consistency check unavailable: Rule consistency response was not valid JSON.",
			conflictsWith: [],
		});
	});
});
