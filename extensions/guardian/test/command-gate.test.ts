import { mkdtempSync, realpathSync, symlinkSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import type { ExtensionContext, ToolCallEvent } from "@earendil-works/pi-coding-agent";
import type { CommandPolicy } from "../preflight/command-policy.js";
import { gateToolCall } from "../preflight/command-gate.js";

const policy: CommandPolicy = {
	defaultDecision: "allow",
	rules: [
		{ id: "hard-reset", pattern: ["git", "reset", "--hard"], decision: "prompt" },
		{
			id: "filter-branch",
			pattern: ["git", "filter-branch"],
			decision: "forbid",
			reason: "Use filter-repo",
		},
	],
};

function ctx(cwd: string): ExtensionContext {
	return { cwd } as ExtensionContext;
}

function event(toolName: string, input: Record<string, unknown>): ToolCallEvent {
	return { toolName, input, toolCallId: "call-1" } as ToolCallEvent;
}

describe("static command and path gate", () => {
	it("sends prompt rules to the model reviewer", () => {
		const result = gateToolCall(event("bash", { command: "git reset --hard HEAD" }), ctx("/tmp"), policy);
		expect(result.action).toBe("review");
	});

	it("keeps hard command blocks outside model control", () => {
		const result = gateToolCall(event("bash", { command: "git filter-branch -- --all" }), ctx("/tmp"), policy);
		expect(result).toMatchObject({ action: "block" });
	});

	it("reviews writes outside the project", () => {
		const cwd = mkdtempSync(join(tmpdir(), "pi-guardian-cwd-"));
		const result = gateToolCall(event("write", { path: join(tmpdir(), "outside.txt") }), ctx(cwd), policy);
		expect(result.action).toBe("review");
	});

	it("reviews a touch command outside the project and grants only its literal path", () => {
		const cwd = mkdtempSync(join(tmpdir(), "pi-guardian-cwd-"));
		const path = join(homedir(), "Projects", "asd");
		const result = gateToolCall(event("bash", { command: "touch ~/Projects/asd" }), ctx(cwd), policy);
		expect(result).toEqual({
			action: "review",
			reason: `Command writes outside the project: ${path}`,
			sandboxAllowWrite: [path],
		});
	});

	it("resolves HOME when checking a shell write path", () => {
		const cwd = mkdtempSync(join(tmpdir(), "pi-guardian-cwd-"));
		const path = join(homedir(), "auto-approval-test");
		const result = gateToolCall(
			event("bash", { command: 'touch "$HOME/auto-approval-test"' }),
			ctx(cwd),
			policy,
		);
		expect(result).toMatchObject({
			action: "review",
			sandboxAllowWrite: [path],
		});
	});

	it("hard-blocks shell writes to protected credential paths", () => {
		const cwd = mkdtempSync(join(tmpdir(), "pi-guardian-cwd-"));
		const result = gateToolCall(event("bash", { command: "touch ~/.ssh/config" }), ctx(cwd), policy);
		expect(result.action).toBe("block");
	});

	it("pins an allowed built-in path to the checked symlink target", () => {
		const cwd = mkdtempSync(join(tmpdir(), "pi-guardian-cwd-"));
		const target = join(cwd, "target.txt");
		const link = join(cwd, "link.txt");
		writeFileSync(target, "safe");
		symlinkSync(target, link);
		const call = event("write", { path: link });

		expect(gateToolCall(call, ctx(cwd), policy)).toEqual({ action: "allow" });
		expect((call.input as { path: string }).path).toBe(realpathSync.native(target));
	});

	for (const toolName of ["read", "write", "edit"]) {
		it(`expands tilde before guarding ${toolName} paths`, () => {
			const cwd = mkdtempSync(join(tmpdir(), "pi-guardian-cwd-"));
			const result = gateToolCall(event(toolName, { path: "~/.ssh/config" }), ctx(cwd), policy);
			expect(result).toMatchObject({ action: "block" });
		});

		it(`normalizes file URLs before guarding ${toolName} paths`, () => {
			const cwd = mkdtempSync(join(tmpdir(), "pi-guardian-cwd-"));
			const url = new URL(`file://${join(homedir(), ".ssh", "config")}`).href;
			const result = gateToolCall(event(toolName, { path: url }), ctx(cwd), policy);
			expect(result).toMatchObject({ action: "block" });
		});
	}

});
