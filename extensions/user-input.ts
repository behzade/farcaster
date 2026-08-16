import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { Effect } from "effect";
import { Type } from "typebox";

const Params = Type.Object({
  question: Type.String({ description: "A short question for the user" }),
  options: Type.Optional(Type.Array(Type.String(), { description: "Optional choices" })),
});

export default function (pi: ExtensionAPI) {
  const requestInput = Effect.fn("UserInput.request")(function* (
    toolCallId: string,
    params: { question: string; options?: string[] },
    ctx: ExtensionContext,
  ) {
    if (!ctx.hasUI) {
      return { content: [{ type: "text" as const, text: "User input is unavailable in this mode" }], isError: true };
    }

    yield* Effect.sync(() => pi.events.emit("approval:requested", {
      kind: "user-input",
      title: "Pi needs input",
      summary: params.question,
      toolName: "request_user_input",
      toolCallId,
      sessionId: ctx.sessionManager.getSessionId(),
      cwd: ctx.cwd,
    }));

    const options = params.options?.filter((option) => option.trim()) ?? [];
    let answer: string | undefined;
    yield* Effect.tryPromise({
      try: (signal) => options.length
        ? ctx.ui.select(params.question, options, { signal })
        : ctx.ui.input(params.question, "Type your answer", { signal }),
      catch: (cause) => cause,
    }).pipe(
      Effect.tap((value) => Effect.sync(() => { answer = value; })),
      Effect.ensuring(Effect.sync(() => pi.events.emit("approval:resolved", {
        kind: "user-input",
        toolName: "request_user_input",
        toolCallId,
        decision: answer === undefined ? "denied" : "allowed",
      }))),
    );

    return {
      content: [{ type: "text" as const, text: answer === undefined ? "User cancelled" : `User answered: ${answer}` }],
      details: { answered: answer !== undefined },
    };
  });

  pi.registerTool({
    name: "request_user_input",
    label: "Ask user",
    description: "Ask the user one clear question when their input is needed to continue.",
    parameters: Params,
    executionMode: "sequential",
    execute(toolCallId, params, signal, _onUpdate, ctx) {
      return Effect.runPromise(requestInput(toolCallId, params, ctx), { signal });
    },
  });
}
