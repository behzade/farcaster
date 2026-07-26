import type { PermissionDecision, ToolPolicyDecision } from "../types.js";

export function parsePolicyDecision(value: unknown): PermissionDecision | "none" | undefined {
	if (typeof value !== "string") return undefined;
	const lowered = value.trim().toLowerCase();
	if (lowered === "allow") return "allow";
	if (lowered === "ask") return "ask";
	if (lowered === "deny") return "deny";
	if (lowered === "none") return "none";
	return undefined;
}

export function normalizePolicyResult(
	decision: unknown,
	reason: unknown,
): ToolPolicyDecision | undefined {
	const parsedDecision = parsePolicyDecision(decision);
	if (!parsedDecision) return undefined;

	if (typeof reason === "string" && reason.trim()) {
		return {
			decision: parsedDecision,
			reason: reason.trim(),
		};
	}

	if (parsedDecision === "none") {
		return {
			decision: "none",
			reason: "No applicable policy rules.",
		};
	}

	return {
		decision: parsedDecision,
		reason: "Policy decision returned by model.",
	};
}
