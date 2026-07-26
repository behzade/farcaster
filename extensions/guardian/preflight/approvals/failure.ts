import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import type { DebugLogger, PreflightFailureDecision, ToolCallSummary } from "../types.js";

export async function handlePreflightFailure(
	toolCalls: ToolCallSummary[],
	reason: string,
	ctx: ExtensionContext,
	logDebug: DebugLogger,
): Promise<PreflightFailureDecision> {
	const toolList = formatToolCallList(toolCalls);
	const failureSummary = `Preflight failed: ${reason}`;
	const message = toolList ? `${failureSummary}\nTool calls: ${toolList}` : failureSummary;

	if (!ctx.hasUI) {
		console.warn(`[bo-pi] ${message}`);
		logDebug(message);
		return { action: "block", reason: failureSummary };
	}

	ctx.ui.notify(message, "warning");
	const selection = await ctx.ui.select("Preflight failed", [
		"Retry preflight",
		"Allow tool call(s) without preflight",
		"Block tool call(s)",
	]);

	if (!selection) {
		return { action: "block", reason: `${failureSummary} (no response from user).` };
	}
	if (selection.startsWith("Retry")) {
		return { action: "retry" };
	}
	if (selection.startsWith("Allow")) {
		return { action: "allow" };
	}
	return { action: "block", reason: `${failureSummary} (blocked by user).` };
}

function formatToolCallList(toolCalls: ToolCallSummary[]): string {
	return toolCalls.map((toolCall) => toolCall.name).join(", ");
}
