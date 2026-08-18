import assert from "node:assert/strict";
import test from "node:test";
import { finalAssistantText } from "./adapter.ts";

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
