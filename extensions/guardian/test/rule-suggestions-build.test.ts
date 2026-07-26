import { describe, expect, it, vi } from "vitest";
import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import type { Api, Context, Model } from "@earendil-works/pi-ai";
import type {
	PreflightConfig,
	RuleContextSnapshot,
	ToolCallSummary,
	ToolCallsContext,
} from "../preflight/types.js";
import { buildRuleSuggestion } from "../preflight/rule-suggestions.js";
import { streamSimple } from "@earendil-works/pi-ai/compat";

vi.mock("@earendil-works/pi-ai/compat", async () => {
	const actual = await vi.importActual<typeof import("@earendil-works/pi-ai/compat")>("@earendil-works/pi-ai/compat");
	return {
		...actual,
		streamSimple: vi.fn(),
	};
});

const model = { provider: "openai-codex", id: "gpt-5.6-luna" } as Model<Api>;

const baseConfig: PreflightConfig = {
	contextMessages: 3,
	explainKey: "ctrl+e",
	ruleSuggestionKey: "ctrl+n",
	model: "current",
	policyModel: "current",
	reasoning: "low",
	approvalMode: "all",
	repeatThreshold: 2,
	debug: false,
};

function createContext(): ExtensionContext {
	return {
		cwd: "/workspace",
		hasUI: false,
		model,
		modelRegistry: {
			find: vi.fn(),
			getApiKeyAndHeaders: vi.fn().mockResolvedValue({ ok: true, apiKey: "test-key" }),
		},
	} as unknown as ExtensionContext;
}

function buildEvent(toolCalls: ToolCallSummary[]): ToolCallsContext {
	const llmContext: Context = {
		systemPrompt: "system",
		messages: [
			{ role: "user", content: "Please inspect the whole project", timestamp: 1 },
			{ role: "assistant", content: [{ type: "text", text: "I will do that" }], timestamp: 2 },
		],
	};
	return { toolCalls, llmContext };
}

function buildRuleSnapshot(): RuleContextSnapshot {
	return {
		tool: "bash",
		policy: {
			global: ["Ask before any destructive command"],
			tool: ["Allow read-only listing commands"],
		},
		permissions: {
			allow: ["Bash(ls:*)"],
			ask: ["Bash(git push*)"],
			deny: ["Bash(rm -rf*)"],
		},
		policyOverrides: ["Bash(ls -la)"],
	};
}

describe("buildRuleSuggestion", () => {
	it("uses tool-call-only context for suggestion generation", async () => {
		const streamMock = vi.mocked(streamSimple);
		streamMock.mockResolvedValue({
			async *[Symbol.asyncIterator]() {
				return;
			},
			result: async () => ({
				content: [
					{
						type: "text",
						text: [
							"Allow read-only listing commands",
							"Ask before commands that modify files",
							"Deny recursive delete commands",
						].join("\n"),
					},
				],
			}),
		} as unknown as Awaited<ReturnType<typeof streamSimple>>);

		const toolCall: ToolCallSummary = {
			id: "call-1",
			name: "bash",
			args: { command: "ls -la" },
		};
		const result = await buildRuleSuggestion(
			buildEvent([toolCall]),
			toolCall,
			{ summary: "List directory contents", destructive: false },
			createContext(),
			baseConfig,
			() => {},
			buildRuleSnapshot(),
			[],
		);

		expect(result.status).toBe("ok");
		expect(streamMock).toHaveBeenCalledTimes(1);
		const streamContext = streamMock.mock.calls[0]?.[1];
		expect(streamContext?.messages).toHaveLength(1);
		expect(streamContext?.messages[0]?.role).toBe("user");
		const prompt = String(streamContext?.messages[0]?.content ?? "");
		expect(prompt).toContain("Existing policy rules (global)");
		expect(prompt).toContain("Deterministic permissions (deny)");
		expect(prompt).toContain("Policy overrides");
	});

	it("filters duplicates against existing policy rules", async () => {
		const streamMock = vi.mocked(streamSimple);
		streamMock.mockResolvedValue({
			async *[Symbol.asyncIterator]() {
				return;
			},
			result: async () => ({
				content: [
					{
						type: "text",
						text: [
							"Allow read-only listing commands",
							"Ask before any destructive command",
							"Deny recursive delete commands",
						].join("\n"),
					},
				],
			}),
		} as unknown as Awaited<ReturnType<typeof streamSimple>>);

		const toolCall: ToolCallSummary = {
			id: "call-1",
			name: "bash",
			args: { command: "ls -la" },
		};
		const result = await buildRuleSuggestion(
			buildEvent([toolCall]),
			toolCall,
			{ summary: "List directory contents", destructive: false },
			createContext(),
			baseConfig,
			() => {},
			buildRuleSnapshot(),
			[],
		);

		expect(result).toEqual({
			status: "ok",
			suggestions: ["Deny recursive delete commands"],
		});
	});
});
