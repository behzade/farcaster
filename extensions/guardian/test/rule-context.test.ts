import { describe, expect, it } from "vitest";
import { buildRuleContextSnapshot, getPolicyRuleCandidates } from "../preflight/rule-context.js";
import type { PermissionsState } from "../preflight/types.js";

describe("rule context snapshot", () => {
	it("includes wildcard and tool-specific context", () => {
		const permissions: PermissionsState = {
			rules: {
				allow: [
					{
						kind: "allow",
						raw: "Bash(ls:*)",
						tool: "bash",
						source: "workspace",
						settingsPath: "/workspace/.pi/preflight/settings.local.json",
						settingsDir: "/workspace/.pi/preflight",
					},
				],
				ask: [],
				deny: [
					{
						kind: "deny",
						raw: "*",
						tool: "*",
						source: "workspace",
						settingsPath: "/workspace/.pi/preflight/settings.local.json",
						settingsDir: "/workspace/.pi/preflight",
					},
				],
			},
			policyRules: [
				{
					tool: "*",
					policy: "Ask before destructive commands",
					source: "workspace",
					settingsPath: "/workspace/.pi/preflight/settings.local.json",
					settingsDir: "/workspace/.pi/preflight",
				},
				{
					tool: "bash",
					policy: "Allow list commands",
					source: "workspace",
					settingsPath: "/workspace/.pi/preflight/settings.local.json",
					settingsDir: "/workspace/.pi/preflight",
				},
			],
			policyOverrides: [
				{
					raw: "*",
					tool: "*",
					source: "workspace",
					settingsPath: "/workspace/.pi/preflight/settings.local.json",
					settingsDir: "/workspace/.pi/preflight",
				},
			],
		};

		const snapshot = buildRuleContextSnapshot("bash", permissions);
		expect(snapshot.policy).toEqual({
			global: ["Ask before destructive commands"],
			tool: ["Allow list commands"],
		});
		expect(snapshot.permissions.allow).toEqual(["Bash(ls:*)"]);
		expect(snapshot.permissions.deny).toEqual(["*"]);
		expect(snapshot.policyOverrides).toEqual(["*"]);
		expect(getPolicyRuleCandidates(snapshot)).toEqual([
			"Ask before destructive commands",
			"Allow list commands",
		]);
	});
});
