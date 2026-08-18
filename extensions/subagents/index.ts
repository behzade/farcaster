import { StringEnum } from "@earendil-works/pi-ai";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { Effect } from "effect";
import { PiSessionFactory } from "./adapter.ts";
import { THINKING_LEVELS, type StartRequest } from "./contract.ts";
import { SubagentCore } from "./core.ts";

const StartParams = Type.Object({
	prompt: Type.String({ minLength: 1 }),
	context: Type.Optional(StringEnum(["fork", "blank"] as const)),
	provider: Type.Optional(Type.String({ minLength: 1 })),
	model: Type.Optional(Type.String({ minLength: 1 })),
	effort: Type.Optional(StringEnum(THINKING_LEVELS)),
});

const SendParams = Type.Object({
	id: Type.String({ minLength: 1 }),
	message: Type.String({ minLength: 1 }),
	mode: Type.Optional(StringEnum(["prompt", "steer"] as const)),
});

const WaitParams = Type.Object({
	ids: Type.Array(Type.String({ minLength: 1 }), { minItems: 1, uniqueItems: true }),
	until: Type.Optional(StringEnum(["first", "all"] as const)),
});

const ControlParams = Type.Object({
	action: StringEnum(["list", "status", "stop"] as const),
	id: Type.Optional(Type.String({ minLength: 1 })),
});

function toolResult(value: unknown) {
	return {
		content: [{ type: "text" as const, text: JSON.stringify(value) }],
		details: value,
	};
}

function runEffect<A>(effect: Effect.Effect<A, Error>, signal?: AbortSignal): Promise<A> {
	return Effect.runPromise(effect, signal ? { signal } : undefined);
}

function startRequest(
	params: {
		prompt: string;
		context?: "fork" | "blank";
		provider?: string;
		model?: string;
		effort?: StartRequest["effort"];
	},
	ctx: ExtensionContext,
): StartRequest {
	return {
		...params,
		cwd: ctx.cwd,
		parentSessionFile: ctx.sessionManager.getSessionFile(),
		parentProvider: ctx.model?.provider,
		parentModel: ctx.model?.id,
		parentEffort: ctx.thinkingLevel,
	};
}

export default function subagentsExtension(pi: ExtensionAPI) {
	const core = new SubagentCore(new PiSessionFactory());

	pi.registerTool({
		name: "subagent_start",
		label: "Start subagent",
		description: "Start one persistent child Pi session. Context defaults to fork; blank is for explicitly independent work. Optional provider/model must be authenticated and effort must be supported by that model.",
		promptSnippet: "Start a plain child Pi session with forked or blank context",
		parameters: StartParams,
		async execute(_toolCallId, params, signal, _onUpdate, ctx) {
			return toolResult(await runEffect(core.start(startRequest(params, ctx)), signal));
		},
	});

	pi.registerTool({
		name: "subagent_send",
		label: "Send to subagent",
		description: "Send another message to a child session. With no mode, steer a running child or prompt an idle child. Prompt mode queues behind current work; steer mode redirects current work.",
		promptSnippet: "Prompt or steer an existing child Pi session",
		parameters: SendParams,
		async execute(_toolCallId, params, signal) {
			return toolResult(await runEffect(core.send(params.id, params.message, params.mode), signal));
		},
	});

	pi.registerTool({
		name: "subagent_wait",
		label: "Wait for subagents",
		description: "Wait for the first or all named child sessions. A new parent user message interrupts waiting successfully without consuming that message; answer it, then wait again.",
		promptSnippet: "Wait interruptibly for child Pi session results",
		parameters: WaitParams,
		async execute(_toolCallId, params) {
			// Parent input aborts the current tool signal before the input event is
			// delivered. Keep this wait alive long enough for notifyUserInput() to
			// resolve it successfully with the resumable interruption result.
			return toolResult(await runEffect(core.wait(params.ids, params.until)));
		},
	});

	pi.registerTool({
		name: "subagent_control",
		label: "Control subagents",
		description: "List child sessions, inspect one status, or stop one child session. status and stop require id.",
		promptSnippet: "List, inspect, or stop child Pi sessions",
		parameters: ControlParams,
		async execute(_toolCallId, params, signal) {
			return toolResult(await runEffect(core.control(params.action, params.id), signal));
		},
	});

	pi.on("input", (event) => {
		if (event.source === "interactive" || event.source === "rpc") core.notifyUserInput();
		return { action: "continue" };
	});

	pi.on("session_shutdown", async () => {
		await Effect.runPromise(core.shutdown());
	});
}
