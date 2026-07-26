import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import type { Api, AssistantMessage, Message, Model, TextContent } from "@earendil-works/pi-ai";
import type { ModelWithKey } from "./types.js";

export async function resolveModelWithApiKey(
	ctx: ExtensionContext,
	modelSetting: { provider: string; id: string } | "current",
): Promise<ModelWithKey | undefined> {
	const candidates: Model<Api>[] = [];
	if (modelSetting === "current") {
		if (ctx.model) candidates.push(ctx.model);
	} else {
		const explicit = ctx.modelRegistry.find(modelSetting.provider, modelSetting.id);
		if (explicit) candidates.push(explicit);
	}

	for (const model of candidates) {
		const auth = await ctx.modelRegistry.getApiKeyAndHeaders(model);
		if (auth.ok && auth.apiKey) {
			return {
				model,
				apiKey: auth.apiKey,
				headers: auth.headers,
				env: auth.env,
			};
		}
	}

	return undefined;
}

export function limitContextMessages(messages: Message[], limit: number): Message[] {
	if (!Number.isFinite(limit)) return messages;
	if (limit < 0) return messages;
	if (limit === 0) return [];
	if (messages.length <= limit) return messages;
	return messages.slice(-limit);
}

export function createUserMessage(text: string): Message {
	return {
		role: "user",
		content: text,
		timestamp: Date.now(),
	};
}

export function extractText(content: AssistantMessage["content"]): string {
	return content
		.filter((block): block is TextContent => block.type === "text")
		.map((block) => block.text)
		.join("\n")
		.trim();
}

export function stripCodeFence(text: string): string {
	if (!text.startsWith("```")) return text;
	const firstNewline = text.indexOf("\n");
	if (firstNewline === -1) return text;
	const withoutFence = text.slice(firstNewline + 1);
	const closingFenceIndex = withoutFence.lastIndexOf("```");
	if (closingFenceIndex === -1) return withoutFence.trim();
	return withoutFence.slice(0, closingFenceIndex).trim();
}

export function extractJsonPayload(text: string): string | undefined {
	const trimmed = text.trim();
	if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
		return trimmed;
	}
	const firstBrace = trimmed.indexOf("{");
	const lastBrace = trimmed.lastIndexOf("}");
	if (firstBrace !== -1 && lastBrace > firstBrace) {
		return trimmed.slice(firstBrace, lastBrace + 1);
	}
	const firstBracket = trimmed.indexOf("[");
	const lastBracket = trimmed.lastIndexOf("]");
	if (firstBracket !== -1 && lastBracket > firstBracket) {
		return trimmed.slice(firstBracket, lastBracket + 1);
	}
	return undefined;
}
