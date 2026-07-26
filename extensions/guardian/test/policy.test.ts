import { describe, expect, it } from "vitest";
import { normalizePolicyResult, parsePolicyDecision } from "../preflight/permissions/policy.js";

describe("policy parsing", () => {
	it("parses decisions case-insensitively", () => {
		expect(parsePolicyDecision("ALLOW")).toBe("allow");
		expect(parsePolicyDecision("ask")).toBe("ask");
		expect(parsePolicyDecision("Deny")).toBe("deny");
		expect(parsePolicyDecision("none")).toBe("none");
	});

	it("normalizes policy result with fallback reason", () => {
		expect(normalizePolicyResult("allow", "ok")).toEqual({ decision: "allow", reason: "ok" });
		expect(normalizePolicyResult("none", "")).toEqual({
			decision: "none",
			reason: "No applicable policy rules.",
		});
		expect(normalizePolicyResult("deny", undefined)).toEqual({
			decision: "deny",
			reason: "Policy decision returned by model.",
		});
	});

	it("rejects invalid decisions", () => {
		expect(parsePolicyDecision("maybe")).toBeUndefined();
		expect(normalizePolicyResult("maybe", "nope")).toBeUndefined();
	});
});
