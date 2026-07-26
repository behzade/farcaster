import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import { describe, expect, it } from "vitest";
import type { ToolCallSummary } from "../preflight/types.js";
import {
	getWorkspacePermissionsPath,
	persistPolicyOverride,
	persistPolicyRule,
	persistWorkspaceRule,
	readPermissionsFile,
} from "../preflight/permissions/persistence.js";

const logDebug = () => {};

function createCtx(cwd: string): ExtensionContext {
	return {
		cwd,
		hasUI: false,
		ui: {
			notify: () => {},
		},
	} as ExtensionContext;
}

describe("permission persistence", () => {
	it("writes workspace allow rules without duplicates", () => {
		const baseDir = mkdtempSync(join(tmpdir(), "bo-pi-"));
		const cwd = join(baseDir, "repo");
		const ctx = createCtx(cwd);
		const toolCall: ToolCallSummary = { id: "1", name: "bash", args: { command: "ls -la" } };

		try {
			persistWorkspaceRule(toolCall, "allow", ctx, logDebug);
			persistWorkspaceRule(toolCall, "allow", ctx, logDebug);

			const settingsPath = getWorkspacePermissionsPath(cwd);
			const settings = readPermissionsFile(settingsPath, logDebug);
			expect(settings?.permissions?.allow).toEqual(["Bash(ls -la)"]);
		} finally {
			rmSync(baseDir, { recursive: true, force: true });
		}
	});

	it("writes policy overrides", () => {
		const baseDir = mkdtempSync(join(tmpdir(), "bo-pi-"));
		const cwd = join(baseDir, "repo");
		const ctx = createCtx(cwd);
		const toolCall: ToolCallSummary = {
			id: "2",
			name: "read",
			args: { path: "/tmp/notes.txt" },
		};

		try {
			persistPolicyOverride(toolCall, ctx, logDebug);

			const settingsPath = getWorkspacePermissionsPath(cwd);
			const settings = readPermissionsFile(settingsPath, logDebug);
			expect(settings?.preflight?.policyOverrides).toEqual(["Read(//tmp/notes.txt)"]);
		} finally {
			rmSync(baseDir, { recursive: true, force: true });
		}
	});

	it("writes custom policy rules to llmRules.<tool>[]", () => {
		const baseDir = mkdtempSync(join(tmpdir(), "bo-pi-"));
		const cwd = join(baseDir, "repo");
		const ctx = createCtx(cwd);
		const toolCall: ToolCallSummary = { id: "3", name: "bash", args: { command: "rm -rf /tmp/x" } };
		try {
			persistPolicyRule(toolCall, "Block destructive shell commands", ctx, logDebug);

			const settingsPath = getWorkspacePermissionsPath(cwd);
			const settings = readPermissionsFile(settingsPath, logDebug);
			expect(settings?.preflight?.llmRules).toEqual({
				bash: ["Block destructive shell commands"],
			});
		} finally {
			rmSync(baseDir, { recursive: true, force: true });
		}
	});

	it("migrates legacy llmRules string[] to current tool bucket on first write", () => {
		const baseDir = mkdtempSync(join(tmpdir(), "bo-pi-"));
		const cwd = join(baseDir, "repo");
		const ctx = createCtx(cwd);
		const toolCall: ToolCallSummary = { id: "4", name: "read", args: { path: "README.md" } };
		const settingsPath = getWorkspacePermissionsPath(cwd);
		try {
			mkdirSync(join(cwd, ".pi", "preflight"), { recursive: true });
			writeFileSync(
				settingsPath,
				JSON.stringify(
					{
						version: 1,
						preflight: {
							llmRules: ["Ask before touching production", "Block dangerous ops"],
						},
					},
					null,
					2,
				),
			);

			persistPolicyRule(toolCall, "Allow reads in docs", ctx, logDebug);

			const settings = readPermissionsFile(settingsPath, logDebug);
			expect(settings?.preflight?.llmRules).toEqual({
				read: ["Allow reads in docs", "Ask before touching production", "Block dangerous ops"],
			});
		} finally {
			rmSync(baseDir, { recursive: true, force: true });
		}
	});

	it("migrates legacy llmRules [{pattern,policy}] by tool name", () => {
		const baseDir = mkdtempSync(join(tmpdir(), "bo-pi-"));
		const cwd = join(baseDir, "repo");
		const ctx = createCtx(cwd);
		const toolCall: ToolCallSummary = { id: "5", name: "bash", args: { command: "ls" } };
		const settingsPath = getWorkspacePermissionsPath(cwd);
		try {
			mkdirSync(join(cwd, ".pi", "preflight"), { recursive: true });
			writeFileSync(
				settingsPath,
				JSON.stringify(
					{
						version: 1,
						preflight: {
							llmRules: [
								{ pattern: "Bash(rm -rf *)", policy: "Block rm -rf" },
								{ pattern: "Read(./docs/**)", policy: "Allow docs reads" },
							],
						},
					},
					null,
					2,
				),
			);

			persistPolicyRule(toolCall, "Allow harmless bash commands", ctx, logDebug);

			const settings = readPermissionsFile(settingsPath, logDebug);
			expect(settings?.preflight?.llmRules).toEqual({
				bash: ["Allow harmless bash commands", "Block rm -rf"],
				read: ["Allow docs reads"],
			});
		} finally {
			rmSync(baseDir, { recursive: true, force: true });
		}
	});
});
