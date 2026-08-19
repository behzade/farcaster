import type { ThinkingLevel as PiThinkingLevel } from "@earendil-works/pi-agent-core";
import { getSupportedThinkingLevels } from "@earendil-works/pi-ai";
import {
	AgentSessionRuntime,
	createAgentSessionFromServices,
	createAgentSessionServices,
	ModelRuntime,
	SessionManager,
} from "@earendil-works/pi-coding-agent";
import type {
	ChildSession,
	ChildSessionFactory,
	SendMode,
	StartRequest,
	ThinkingLevel,
} from "./contract.ts";

function finalAssistantText(messages: readonly unknown[]): string {
	for (let index = messages.length - 1; index >= 0; index -= 1) {
		const message = messages[index] as { role?: string; content?: unknown };
		if (message.role !== "assistant" || !Array.isArray(message.content)) continue;
		return message.content
			.filter((part): part is { type: "text"; text: string } => {
				if (!part || typeof part !== "object") return false;
				const value = part as { type?: unknown; text?: unknown };
				return value.type === "text" && typeof value.text === "string";
			})
			.map((part) => part.text)
			.join("");
	}
	return "";
}

function selectModel(
	available: readonly ReturnType<ModelRuntime["getModel"]>[],
	request: StartRequest,
) {
	const models = available.filter((model) => model !== undefined);
	if (request.provider || request.model) {
		const matches = models.filter((model) =>
			(!request.provider || model.provider === request.provider)
			&& (!request.model || model.id === request.model));
		const selected = matches.find((model) => model.provider === request.parentProvider) ?? matches[0];
		if (selected) return selected;
		const requested = [request.provider, request.model].filter(Boolean).join("/");
		throw new Error(`No authenticated model matches ${requested}`);
	}
	const selected = models.find((model) =>
		model.provider === request.parentProvider && model.id === request.parentModel)
		?? models[0];
	if (!selected) throw new Error("No authenticated model is available");
	return selected;
}

export class PiSessionFactory implements ChildSessionFactory {
	#runtime: Promise<ModelRuntime> | undefined;

	async create(request: StartRequest): Promise<ChildSession> {
		const modelRuntime = await (this.#runtime ??= ModelRuntime.create());
		const available = await modelRuntime.getAvailable(request.provider);
		const model = selectModel(available, request);
		const supported = getSupportedThinkingLevels(model) as ThinkingLevel[];
		const requestedEffort = request.effort ?? request.parentEffort ?? "medium";
		if (request.effort && !supported.includes(request.effort)) {
			throw new Error(
				`${model.provider}/${model.id} does not support effort ${request.effort}; supported: ${supported.join(", ")}`,
			);
		}
		const effort = supported.includes(requestedEffort)
			? requestedEffort
			: (supported.includes("medium") ? "medium" : (supported[0] ?? "off"));
		const context = request.context ?? "fork";
		const manager = context === "fork"
			? forkBeforeActiveToolCall(request.parentSessionFile!, request.cwd)
			: SessionManager.create(request.cwd, undefined, { parentSession: request.parentSessionFile });
		if (context === "fork") {
			recordChildRuntimeIdentity(manager, model.provider, model.id, effort);
		}
		const services = await createAgentSessionServices({
			cwd: request.cwd,
			modelRuntime,
		});
		const sessionStartEvent = {
			type: "session_start" as const,
			reason: context === "fork" ? "fork" as const : "new" as const,
			previousSessionFile: request.parentSessionFile,
		};
		const { session } = await createAgentSessionFromServices({
			services,
			model,
			thinkingLevel: effort as PiThinkingLevel,
			sessionManager: manager,
			sessionStartEvent,
			excludeTools: ["subagent_start", "subagent_send", "subagent_control"],
		});
		const runtime = new AgentSessionRuntime(
			session,
			services,
			async () => { throw new Error("Subagent runtime replacement is unsupported"); },
			services.diagnostics,
		);
		try {
			// SDK-created sessions are unbound until their host mode initializes the
			// extension runner. Children are headless, but still need lifecycle events
			// so Guardian and every other extension can initialize isolated state.
			await session.bindExtensions({ mode: "print" });
		} catch (error) {
			await runtime.dispose();
			throw error;
		}
		const sessionFile = session.sessionFile;
		if (!sessionFile) {
			await runtime.dispose();
			throw new Error("Subagent session was not persisted");
		}

		return {
			id: session.sessionId,
			sessionFile,
			provider: model.provider,
			model: model.id,
			effort,
			isStreaming: () => session.isStreaming,
			async run(prompt: string) {
				await session.prompt(prompt);
				await session.waitForIdle();
				return finalAssistantText(session.messages);
			},
			async send(message: string, mode: SendMode) {
				if (mode === "steer" && session.isStreaming) {
					await session.steer(message);
					return;
				}
				await session.prompt(message, session.isStreaming
					? { streamingBehavior: "followUp" }
					: undefined);
			},
			abort: () => session.abort(),
			dispose: () => runtime.dispose(),
		};
	}
}

function recordChildRuntimeIdentity(
	manager: SessionManager,
	provider: string,
	model: string,
	effort: ThinkingLevel,
): void {
	manager.appendModelChange(provider, model);
	manager.appendThinkingLevelChange(effort);
}

function forkBeforeActiveToolCall(parentSessionFile: string, cwd: string): SessionManager {
	const manager = SessionManager.forkFrom(parentSessionFile, cwd);
	const leaf = manager.getLeafEntry();
	if (leaf?.type === "message" && leaf.message.role === "assistant") {
		const startsChild = Array.isArray(leaf.message.content) && leaf.message.content.some((part) =>
			part.type === "toolCall" && part.name === "subagent_start");
		if (startsChild) {
			if (leaf.parentId) manager.branch(leaf.parentId);
			else manager.resetLeaf();
		}
	}
	return manager;
}

export { finalAssistantText, forkBeforeActiveToolCall, recordChildRuntimeIdentity };
