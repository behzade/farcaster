import { describe, expect, it } from "vitest";
import {
	canonicalizeRuleText,
	normalizeRuleSuggestionLine,
	normalizeRuleSuggestions,
} from "../preflight/rule-suggestions.js";

describe("rule suggestion normalization", () => {
	it("capitalizes suggestion lines that start with lowercase", () => {
		const normalized = normalizeRuleSuggestionLine("allow only read-only commands");
		expect(normalized).toBe("Allow only read-only commands");
	});

	it("drops heading-only lines", () => {
		const normalized = normalizeRuleSuggestionLine(
			"Here are three policy rule suggestions for this type of operation:",
		);
		expect(normalized).toBeUndefined();
	});

	it("normalizes policy verb casing", () => {
		expect(normalizeRuleSuggestionLine("ALLOW read-only shell commands")).toBe(
			"Allow read-only shell commands",
		);
		expect(normalizeRuleSuggestionLine("ask before any write command")).toBe(
			"Ask before any write command",
		);
		expect(normalizeRuleSuggestionLine("DENY dangerous shell operations")).toBe(
			"Deny dangerous shell operations",
		);
	});

	it("drops lines without policy verb prefix", () => {
		expect(normalizeRuleSuggestionLine("<Function_calls>")).toBeUndefined();
		expect(normalizeRuleSuggestionLine("Tool call details")).toBeUndefined();
	});

	it("filters heading and keeps cleaned suggestions", () => {
		const normalized = normalizeRuleSuggestions(
			[
				"Here are three policy rule suggestions for this type of operation:",
				"1. allow list commands",
				"2. Ask before any write command",
				"3. block dangerous shell operations",
			].join("\n"),
			[],
		);

		expect(normalized).toEqual([
			"Allow list commands",
			"Ask before any write command",
			"Deny dangerous shell operations",
		]);
	});

	it("filters near-exact duplicates from existing rules", () => {
		const normalized = normalizeRuleSuggestions(
			[
				"Allow list commands",
				"Ask before writes",
				"Deny recursive delete commands",
			].join("\n"),
			[],
			["  allow   list commands.", "ask before writes"],
		);

		expect(normalized).toEqual(["Deny recursive delete commands"]);
	});

	it("canonicalizes whitespace and trailing punctuation", () => {
		expect(canonicalizeRuleText("  Ask   before writes. ")).toBe("ask before writes");
	});
});
