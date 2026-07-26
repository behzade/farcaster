import { describe, expect, it } from "vitest";
import type { ExtensionContext, ToolCallEvent } from "@earendil-works/pi-coding-agent";
import { buildActionFingerprint } from "../preflight/guardian-rules.js";
import {
	findConcurrentFileRace,
	matchesReviewedInput,
	resolveSiblingToolCalls,
} from "../preflight/index.js";

describe("sibling tool call discovery", () => {
	it("reads the full current assistant batch and keeps the live current input", () => {
		const ctx = {
			sessionManager: {
				getBranch: () => [
					{
						type: "message",
						message: {
							role: "assistant",
							content: [
								{
									type: "toolCall",
									id: "call-allow",
									name: "bash",
									arguments: { command: "touch /tmp/approved-probe" },
								},
								{
									type: "toolCall",
									id: "call-deny",
									name: "bash",
									arguments: { command: "touch /tmp/stale-value" },
								},
							],
						},
					},
				],
			},
		} as unknown as ExtensionContext;
		const event = {
			toolCallId: "call-deny",
			toolName: "bash",
			input: { command: "touch /tmp/rejected-probe" },
		} as ToolCallEvent;

		expect(resolveSiblingToolCalls(event, ctx, () => {})).toEqual([
			{
				id: "call-allow",
				name: "bash",
				args: { command: "touch /tmp/approved-probe" },
			},
			{
				id: "call-deny",
				name: "bash",
				args: { command: "touch /tmp/rejected-probe" },
			},
		]);
	});

	it("rejects a cached sibling verdict when the live input changes", () => {
		const cwd = "/tmp/project";
		const reviewed = {
			id: "call-2",
			name: "bash",
			args: { command: "touch /tmp/approved-probe" },
		};
		const fingerprint = buildActionFingerprint(reviewed, cwd).fingerprint;

		expect(matchesReviewedInput(reviewed, cwd, fingerprint)).toBe(true);
		expect(
			matchesReviewedInput(
				{
					...reviewed,
					args: { command: "touch /tmp/changed-after-review" },
				},
				cwd,
				fingerprint,
			),
		).toBe(false);
	});

	it("blocks a built-in file call beside a sibling shell file change", () => {
		const ctx = {
			sessionManager: {
				getBranch: () => [
					{
						type: "message",
						message: {
							role: "assistant",
							content: [
								{
									type: "toolCall",
									id: "call-link",
									name: "bash",
									arguments: {
										command:
											"python3 -c 'import os; os.rename(\"safe\", \"old\"); os.symlink(os.path.expanduser(\"~/.ssh\"), \"safe\")'",
									},
								},
								{
									type: "toolCall",
									id: "call-write",
									name: "write",
									arguments: { path: "redirected/config", content: "unsafe" },
								},
							],
						},
					},
				],
			},
		} as unknown as ExtensionContext;
		const event = {
			toolCallId: "call-write",
			toolName: "write",
			input: { path: "redirected/config", content: "unsafe" },
		} as ToolCallEvent;

		expect(findConcurrentFileRace(event, ctx, () => {})).toContain(
			"run these actions in order",
		);
	});
});
