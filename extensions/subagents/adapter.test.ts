import assert from "node:assert/strict";
import test from "node:test";
import { SessionManager } from "@earendil-works/pi-coding-agent";
import { finalAssistantText, recordChildRuntimeIdentity } from "./adapter.ts";

test("forked children persist their selected runtime identity before the first prompt", () => {
	const manager = SessionManager.inMemory("/project");
	manager.appendModelChange("openai-codex", "gpt-5.6-sol");
	manager.appendThinkingLevelChange("medium");

	recordChildRuntimeIdentity(manager, "opencode-go", "kimi-k3", "high");

	assert.deepEqual(
		manager.getBranch().slice(-2).map((entry) => {
			if (entry.type === "model_change") {
				return { type: entry.type, provider: entry.provider, modelId: entry.modelId };
			}
			if (entry.type === "thinking_level_change") {
				return { type: entry.type, thinkingLevel: entry.thinkingLevel };
			}
			return { type: entry.type };
		}),
		[
			{ type: "model_change", provider: "opencode-go", modelId: "kimi-k3" },
			{ type: "thinking_level_change", thinkingLevel: "high" },
		],
	);
});

test("adapter returns every text part of the final assistant message verbatim", () => {
	const output = finalAssistantText([
		{ role: "assistant", content: [{ type: "text", text: "older" }] },
		{ role: "user", content: [{ type: "text", text: "continue" }] },
		{
			role: "assistant",
			content: [
				{ type: "thinking", thinking: "not returned" },
				{ type: "text", text: "Should I continue?\n" },
				{ type: "toolCall", name: "read", arguments: {} },
				{ type: "text", text: "Final line." },
			],
		},
	]);
	assert.equal(output, "Should I continue?\nFinal line.");
});
