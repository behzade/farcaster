import { describe, expect, it, vi } from "vitest";
import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import type { Api, Context, Model } from "@earendil-works/pi-ai";
import type { PreflightConfig, ToolCallSummary, ToolCallsContext } from "../preflight/types.js";
import {
	buildPreflightMetadata,
	findExplicitApprovalDirective,
	normalizePreflight,
	parsePreflightResponse,
} from "../preflight/preflight.js";
import { streamSimple } from "@earendil-works/pi-ai/compat";

vi.mock("@earendil-works/pi-ai/compat", async () => {
	const actual = await vi.importActual<typeof import("@earendil-works/pi-ai/compat")>("@earendil-works/pi-ai/compat");
	return {
		...actual,
		streamSimple: vi.fn(),
	};
});

const baseConfig: PreflightConfig = {
	contextMessages: 1,
	explainKey: "ctrl+e",
	ruleSuggestionKey: "ctrl+n",
	model: "current",
	policyModel: "current",
	approvalMode: "all",
	repeatThreshold: 2,
	debug: true,
};

const guardianAllow = {
	riskLevel: "low",
	userAuthorization: "high",
	outcome: "allow",
	rationale: "Narrow read-only action.",
} as const;

const model = { provider: "openai-codex", id: "gpt-5.6-luna" } as Model<Api>;

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

function buildEvent(toolCalls: ToolCallSummary[], latestUser?: string): ToolCallsContext {
	const llmContext: Context = {
		systemPrompt: "system",
		messages: latestUser
			? [{ role: "user", content: latestUser, timestamp: Date.now() }]
			: [],
	};
	return { toolCalls, llmContext };
}

function createStreamResponse(text: string): Awaited<ReturnType<typeof streamSimple>> {
	return {
		async *[Symbol.asyncIterator]() {
			return;
		},
		result: async () => ({
			content: [
				{
					type: "text",
					text,
				},
			],
		}),
	} as unknown as Awaited<ReturnType<typeof streamSimple>>;
}

describe("preflight parsing", () => {
	it("detects passive deny and mixed approval-test directives", () => {
		expect(
			findExplicitApprovalDirective([
				{ role: "user", content: "Somewhere in home; it should be rejected.", timestamp: Date.now() },
			]),
		).toBe("deny");
		expect(
			findExplicitApprovalDirective([
				{
					role: "user",
					content: "The approval model should approve one and reject the other.",
					timestamp: Date.now(),
				},
			]),
		).toBe("mixed");
		expect(
			findExplicitApprovalDirective([
				{ role: "user", content: "The parser should reject invalid JSON.", timestamp: Date.now() },
			]),
		).toBeUndefined();
	});

	it("parses JSON response with intrinsic and policy", () => {
		const parsed = parsePreflightResponse(
			`{"call-1":{"intrinsic":{"summary":"List files","destructive":false,"riskLevel":"low","userAuthorization":"high","outcome":"allow","rationale":"Narrow read-only action."},"policy":{"decision":"ask","reason":"Needs confirmation"}}}`,
		);
		expect(parsed?.["call-1"]).toBeDefined();
	});

	it("requires intrinsic metadata for every tool call", () => {
		const toolCalls: ToolCallSummary[] = [
			{ id: "call-1", name: "bash", args: { command: "ls" } },
			{ id: "call-2", name: "write", args: { path: "note.txt", content: "hi" } },
		];
		const parsed = parsePreflightResponse(
			`{"call-1":{"intrinsic":{"summary":"List files","destructive":false,"riskLevel":"low","userAuthorization":"high","outcome":"allow","rationale":"Narrow read-only action."},"policy":{"decision":"none","reason":"No rules"}}}`,
		);
		expect(normalizePreflight(parsed, toolCalls, { "call-1": [], "call-2": [] })).toBeUndefined();
	});

	it("sanitizes summaries and falls back policy to none on invalid policy payload", () => {
		const toolCalls: ToolCallSummary[] = [{ id: "call-1", name: "bash", args: { command: "ls" } }];
		const parsed = parsePreflightResponse(
			`{"call-1":{"intrinsic":{"summary":"Run bash to list files","destructive":false,"riskLevel":"low","userAuthorization":"high","outcome":"allow","rationale":"Narrow read-only action."},"policy":{"decision":"maybe","reason":"oops"}}}`,
		);
		const normalized = normalizePreflight(parsed, toolCalls, { "call-1": ["ask before running bash"] });
		expect(normalized?.metadata["call-1"].summary).toBe("List files");
		expect(normalized?.policyDecisions["call-1"]).toEqual({
			decision: "none",
			reason: "Policy response missing or invalid; fallback applied.",
		});
	});
});

describe("buildPreflightMetadata", () => {
	it("reviews mixed sibling calls in one model request and keeps each verdict", async () => {
		const streamMock = vi.mocked(streamSimple);
		streamMock.mockReset();
		streamMock.mockResolvedValue(
			createStreamResponse(
				JSON.stringify({
					"call-allow": {
						intrinsic: {
							summary: "Create approved probe",
							destructive: true,
							...guardianAllow,
						},
						policy: { decision: "none", reason: "No matching rule" },
					},
					"call-deny": {
						intrinsic: {
							summary: "Create rejected probe",
							destructive: true,
							riskLevel: "low",
							userAuthorization: "low",
							outcome: "deny",
							rationale: "The user named this action for rejection.",
						},
						policy: { decision: "none", reason: "No matching rule" },
					},
				}),
			),
		);
		const toolCalls: ToolCallSummary[] = [
			{ id: "call-allow", name: "bash", args: { command: "touch /tmp/approved-probe" } },
			{ id: "call-deny", name: "bash", args: { command: "touch /tmp/rejected-probe" } },
		];

		const result = await buildPreflightMetadata(
			buildEvent(
				toolCalls,
				"Touch both files. The approval model should approve approved-probe and reject rejected-probe.",
			),
			{ "call-allow": [], "call-deny": [] },
			createContext(),
			baseConfig,
			() => {},
		);

		expect(streamMock).toHaveBeenCalledTimes(1);
		expect(result.status).toBe("ok");
		if (result.status === "ok") {
			expect(result.metadata["call-allow"].outcome).toBe("allow");
			expect(result.metadata["call-deny"].outcome).toBe("deny");
		}
	});

	it("uses one LLM call for intrinsic+policy and logs prompt/raw response", async () => {
		const streamMock = vi.mocked(streamSimple);
		streamMock.mockReset();
		streamMock.mockResolvedValue(
			createStreamResponse(
				JSON.stringify({
					"call-1": {
						intrinsic: { summary: "List directory", destructive: false, ...guardianAllow },
						policy: { decision: "ask", reason: "Needs confirmation" },
					},
				}),
			),
		);

		const logs: string[] = [];
		const toolCalls: ToolCallSummary[] = [{ id: "call-1", name: "bash", args: { command: "ls" } }];
		const result = await buildPreflightMetadata(
			buildEvent(toolCalls),
			{ "call-1": ["Ask before shell commands"] },
			createContext(),
			baseConfig,
			(message) => logs.push(message),
		);

		expect(streamMock).toHaveBeenCalledTimes(1);
		expect(result.status).toBe("ok");
		if (result.status === "ok") {
			expect(result.metadata["call-1"].summary).toBe("List directory");
			expect(result.policyDecisions["call-1"]).toEqual({
				decision: "ask",
				reason: "Needs confirmation",
			});
		}
		expect(logs.some((line) => line.includes("Preflight prompt"))).toBe(true);
		expect(logs.some((line) => line.includes("Preflight raw response"))).toBe(true);
	});

	it("retries twice when parsing/normalization fails and eventually succeeds", async () => {
		const streamMock = vi.mocked(streamSimple);
		streamMock.mockReset();
		streamMock
			.mockResolvedValueOnce(createStreamResponse("not-json"))
			.mockResolvedValueOnce(createStreamResponse(JSON.stringify({ unexpected: true })))
			.mockResolvedValueOnce(
				createStreamResponse(
					JSON.stringify({
						"call-1": {
							intrinsic: { summary: "List directory", destructive: false, ...guardianAllow },
							policy: { decision: "none", reason: "No rules" },
						},
					}),
				),
			);

		const logs: string[] = [];
		const toolCalls: ToolCallSummary[] = [{ id: "call-1", name: "bash", args: { command: "ls" } }];
		const result = await buildPreflightMetadata(
			buildEvent(toolCalls),
			{ "call-1": [] },
			createContext(),
			baseConfig,
			(message) => logs.push(message),
		);

		expect(streamMock).toHaveBeenCalledTimes(3);
		expect(result.status).toBe("ok");
		expect(logs.some((line) => line.includes("Retrying preflight (attempt 2/3)"))).toBe(true);
		expect(logs.some((line) => line.includes("Retrying preflight (attempt 3/3)"))).toBe(true);
	});

	it("uses approval wording only as a routing hint and leaves the verdict to the reviewer", async () => {
		const streamMock = vi.mocked(streamSimple);
		streamMock.mockReset();
		streamMock.mockResolvedValue(
			createStreamResponse(
				JSON.stringify({
					"call-1": {
						intrinsic: { summary: "Create outside file", destructive: true, ...guardianAllow },
						policy: { decision: "none", reason: "No rules" },
					},
				}),
			),
		);
		const toolCalls: ToolCallSummary[] = [
			{ id: "call-1", name: "bash", args: { command: "touch ~/Projects/asd" } },
		];

		const result = await buildPreflightMetadata(
			buildEvent(toolCalls, "Try it, but the approval should reject this."),
			{ "call-1": [] },
			createContext(),
			baseConfig,
			() => undefined,
		);

		expect(result.status).toBe("ok");
		if (result.status === "ok") {
			expect(result.metadata["call-1"]).toMatchObject({
				userAuthorization: "high",
				outcome: "allow",
			});
		}
	});

	it("returns error after exhausting retry budget", async () => {
		const streamMock = vi.mocked(streamSimple);
		streamMock.mockReset();
		streamMock
			.mockResolvedValueOnce(createStreamResponse("not-json"))
			.mockResolvedValueOnce(createStreamResponse("still-not-json"))
			.mockResolvedValueOnce(createStreamResponse("nope"));

		const toolCalls: ToolCallSummary[] = [{ id: "call-1", name: "bash", args: { command: "ls" } }];
		const result = await buildPreflightMetadata(
			buildEvent(toolCalls),
			{ "call-1": [] },
			createContext(),
			baseConfig,
			() => undefined,
		);

		expect(streamMock).toHaveBeenCalledTimes(3);
		expect(result).toEqual({
			status: "error",
			reason: "Preflight response was not valid JSON.",
		});
	});
});
