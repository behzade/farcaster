import assert from "node:assert/strict";
import test from "node:test";
import type { RunSnapshot } from "./contract.ts";
import { completionReprompt, summarizeRuns } from "./index.ts";

function run(overrides: Partial<RunSnapshot> = {}): RunSnapshot {
	return {
		id: "child-1",
		sessionId: "session-1",
		sessionFile: "/tmp/session.jsonl",
		status: "idle",
		context: "fork",
		provider: "provider",
		model: "model",
		effort: "low",
		...overrides,
	};
}

test("summarizeRuns keeps completed subagent output to one bounded line", () => {
	const summary = summarizeRuns([run({ output: `Finding summary\n${"detail ".repeat(100)}` })]);
	assert.equal(summary, "child-1 (idle): Finding summary");
});

test("summarizeRuns keeps failures visible and truncates long first lines", () => {
	const summary = summarizeRuns([run({ status: "failed", error: "x".repeat(200) })]);
	assert.equal(summary.length, "child-1 (failed): ".length + 160);
	assert.match(summary, /^child-1 \(failed\): x+\.\.\.$/);
});

test("completionReprompt requires a user-visible continuation after the result", () => {
	const prompt = completionReprompt([run({ output: "The validation passed." })]);
	assert.match(prompt, /Subagent child-1 \(idle\) returned:\nThe validation passed\./);
	assert.match(prompt, /Continue the interrupted task now\./);
	assert.match(prompt, /provide a user-visible response before ending your turn\.$/);
});
