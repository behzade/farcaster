import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import { streamSimple } from "@earendil-works/pi-ai/compat";
import type { Context } from "@earendil-works/pi-ai";
import { formatContextLabel } from "./config.js";
import type {
	DebugLogger,
	ExplanationAttempt,
	PreflightConfig,
	ToolPreflightMetadata,
	ToolCallSummary,
	ToolCallsContext,
} from "./types.js";
import {
	createUserMessage,
	extractText,
	limitContextMessages,
	resolveModelWithApiKey,
	stripCodeFence,
} from "./llm-utils.js";

export async function buildToolCallExplanation(
	event: ToolCallsContext,
	toolCall: ToolCallSummary,
	metadata: ToolPreflightMetadata | undefined,
	ctx: ExtensionContext,
	config: PreflightConfig,
	logDebug: DebugLogger,
	signal?: AbortSignal,
): Promise<ExplanationAttempt> {
	const modelWithKey = await resolveModelWithApiKey(ctx, config.model);
	if (!modelWithKey) {
		const reason = "No model or API key available for explanation.";
		logDebug(`Explanation failed: ${reason}`);
		return { status: "error", reason };
	}

	const contextLabel = formatContextLabel(config.contextMessages);
	logDebug(`Explanation model: ${modelWithKey.model.provider}/${modelWithKey.model.id}.`);
	logDebug(`Explanation context: ${contextLabel} messages.`);

	const instruction = buildExplainPrompt(toolCall, metadata);
	const trimmedContext = limitContextMessages(event.llmContext.messages, config.contextMessages);
	const explainContext: Context = {
		...event.llmContext,
		messages: [...trimmedContext, createUserMessage(instruction)],
	};

	logDebug(`Explanation prompt:\n${instruction}`);
	logDebug(`Explanation context messages:\n${JSON.stringify(explainContext.messages, null, 2)}`);

	try {
		const response = await streamSimple(modelWithKey.model, explainContext, {
			apiKey: modelWithKey.apiKey,
			headers: modelWithKey.headers,
			env: modelWithKey.env,
			signal,
		});
		for await (const _ of response) {
			// Drain stream to completion.
		}
		const result = await response.result();
		const rawText = extractText(result.content);
		logDebug(`Explanation raw response:\n${rawText ?? ""}`);
		const text = normalizeExplanation(rawText);
		if (!text) {
			const reason = "Explanation response was empty.";
			logDebug(`Explanation failed: ${reason}`);
			return { status: "error", reason };
		}
		logDebug(`Explanation generated for ${toolCall.name}.`);
		return { status: "ok", text };
	} catch (error) {
		if (signal?.aborted) {
			return { status: "error", reason: "Explanation request cancelled." };
		}
		const message = error instanceof Error ? error.message : String(error);
		const reason = message ? `Explanation request failed: ${message}` : "Explanation request failed.";
		logDebug(`Explanation failed: ${reason}`);
		return { status: "error", reason };
	}
}

function buildExplainPrompt(
	toolCall: ToolCallSummary,
	metadata: ToolPreflightMetadata | undefined,
): string {
	const summary = metadata?.summary ?? "Review requested action";
	const destructive = metadata?.destructive ?? false;
	const scopeLine = metadata?.scope?.length ? `Scope: ${metadata.scope.join(", ")}` : undefined;

	const lines = [
		"You are explaining a tool call before execution.",
		"Write two short paragraphs without headings, labels, or bullet points.",
		"First paragraph: what will happen (include concrete details from the tool call).",
		"Second paragraph: why this is needed for the user's request, citing relevant context details.",
		"End with a single risk line formatted exactly: '<Level> risk: <reason>'.",
		"Use Level = Low, Med, or High.",
		"You may mention tool names and key arguments like file paths or commands.",
		"Avoid markdown and do not include JSON.",
		"If context details are missing, say so explicitly in the second paragraph.",
		`Summary: ${summary}`,
		`Destructive: ${destructive ? "yes" : "no"}.`,
	];
	if (scopeLine) lines.push(scopeLine);
	lines.push("Tool call:", JSON.stringify(toolCall, null, 2));
	return lines.join("\n");
}

function normalizeExplanation(text: string | undefined): string | undefined {
	if (!text) return undefined;
	const cleaned = stripCodeFence(text.trim());
	const normalized = cleaned.trim();
	return normalized ? normalized : undefined;
}
