import { StringEnum } from "@earendil-works/pi-ai";
import { keyHint, type ExtensionAPI, type ExtensionContext } from "@earendil-works/pi-coding-agent";
import { Text } from "@earendil-works/pi-tui";
import { Type } from "typebox";
import { Effect } from "effect";
import { PiSessionFactory } from "./adapter.ts";
import { THINKING_LEVELS, type RunSnapshot, type StartRequest } from "./contract.ts";
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

export function summarizeRuns(runs: readonly RunSnapshot[]): string {
	return runs.map((run) => {
		const body = run.output ?? run.error ?? "Subagent finished without text.";
		const firstLine = body.split(/\r?\n/, 1)[0]?.trim() || "Subagent finished without text.";
		const preview = firstLine.length > 160 ? `${firstLine.slice(0, 157)}...` : firstLine;
		return `${run.id} (${run.status}): ${preview}`;
	}).join("\n");
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
	pi.registerMessageRenderer("subagent-result", (message, { expanded, outputPad }, theme) => {
		if (expanded) return new Text(message.content, outputPad, 0);
		const runs = (message.details as { runs?: RunSnapshot[] } | undefined)?.runs ?? [];
		let text = runs.length > 0 ? summarizeRuns(runs) : "Subagent finished.";
		text += theme.fg("dim", `\n${keyHint("app.tools.expand", "to expand")}`);
		return new Text(text, outputPad, 0);
	});

	let settled: RunSnapshot[] = [];
	let flushTimer: ReturnType<typeof setTimeout> | undefined;
	const flushSettled = () => {
		flushTimer = undefined;
		const batch = settled;
		settled = [];
		if (batch.length === 0) return;
		const content = batch.map((run) => {
			const body = run.output ?? run.error ?? "Subagent finished without text.";
			return `Subagent ${run.id} (${run.status}) returned:\n${body}`;
		}).join("\n\n");
		pi.sendMessage({
			customType: "subagent-result",
			content,
			display: true,
			details: { runs: batch },
		}, { triggerTurn: true, deliverAs: "steer" });
	};
	const core = new SubagentCore(new PiSessionFactory(), (snapshot) => {
		settled.push(snapshot);
		flushTimer ??= setTimeout(flushSettled, 25);
	});

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
		name: "subagent_control",
		label: "Control subagents",
		description: "List child sessions, inspect one status, or stop one child session. status and stop require id. Child completion is delivered automatically. Do not poll or sleep; use status only to debug or intervene.",
		promptSnippet: "List, inspect, or stop child Pi sessions",
		parameters: ControlParams,
		async execute(_toolCallId, params, signal) {
			return toolResult(await runEffect(core.control(params.action, params.id), signal));
		},
	});

	pi.on("session_shutdown", async () => {
		if (flushTimer) clearTimeout(flushTimer);
		flushTimer = undefined;
		settled = [];
		await Effect.runPromise(core.shutdown());
	});
}
