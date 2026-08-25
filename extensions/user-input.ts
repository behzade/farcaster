import {
  createLocalBashOperations,
  type ExtensionAPI,
  type ExtensionContext,
} from "@earendil-works/pi-coding-agent";
import { Effect } from "effect";
import { type Static, Type } from "typebox";
import { executeHostScript, formatHostScriptPrompt } from "./lib/user-input-core.ts";

const Params = Type.Union([
  Type.Object({
    question: Type.String({ description: "A short question for the user" }),
    options: Type.Array(Type.String({ minLength: 1 }), {
      description: "Choices the user can select from",
      minItems: 1,
    }),
  }, { additionalProperties: false }),
  Type.Object({
    question: Type.String({ description: "Why the user needs to run this script" }),
    script: Type.String({
      description: "Exact shell script to approve and execute once outside the sandbox",
      minLength: 1,
    }),
  }, { additionalProperties: false }),
]);

type InputParams = Static<typeof Params>;

export default function (pi: ExtensionAPI) {
  const hostBash = createLocalBashOperations();

  const requestInput = Effect.fn("UserInput.request")(function* (
    toolCallId: string,
    params: InputParams,
    ctx: ExtensionContext,
  ) {
    if (!ctx.hasUI) {
      return { content: [{ type: "text" as const, text: "User input is unavailable in this mode" }], isError: true };
    }

    if ("script" in params) {
      let approved = false;
      yield* Effect.sync(() => pi.events.emit("approval:requested", {
        kind: "command",
        title: "Run outside sandbox",
        summary: params.question,
        toolName: "request_user_input",
        toolCallId,
        sessionId: ctx.sessionManager.getSessionId(),
        cwd: ctx.cwd,
      }));
      yield* Effect.tryPromise({
        try: (signal) => ctx.ui.confirm(
          "Run outside sandbox?",
          formatHostScriptPrompt(params.question, ctx.cwd, params.script),
          { signal },
        ),
        catch: (cause) => cause,
      }).pipe(
        Effect.tap((value) => Effect.sync(() => { approved = value; })),
        Effect.ensuring(Effect.sync(() => pi.events.emit("approval:resolved", {
          kind: "command",
          toolName: "request_user_input",
          toolCallId,
          decision: approved ? "allowed" : "denied",
        }))),
      );

      if (!approved) {
        return {
          content: [{ type: "text" as const, text: "User declined to run the script outside the sandbox." }],
          details: { status: "declined" as const },
        };
      }

      const result = yield* Effect.tryPromise({
        try: (signal) => executeHostScript(hostBash, params.script, ctx.cwd, signal),
        catch: (cause) => cause,
      });
      const heading = result.status === "success"
        ? "Host script succeeded (exit 0)."
        : `Host script failed${result.exitCode === null ? "" : ` (exit ${result.exitCode})`}.`;
      return {
        content: [{
          type: "text" as const,
          text: `${heading}\n${result.output || "(no output)"}`,
        }],
        details: {
          status: result.status,
          exitCode: result.exitCode,
          truncated: result.truncated,
        },
        isError: result.status === "failure",
      };
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

    let answer: string | undefined;
    yield* Effect.tryPromise({
      try: (signal) => ctx.ui.select(params.question, params.options, { signal }),
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
    description: "Ask the user to choose an answer or approve one script to execute outside the sandbox.",
    promptSnippet: "Use script mode only when request_access cannot grant the required operation. The user must explicitly approve it; the script runs once and its result is returned.",
    parameters: Params,
    executionMode: "sequential",
    execute(toolCallId, params, signal, _onUpdate, ctx) {
      return Effect.runPromise(requestInput(toolCallId, params, ctx), { signal });
    },
  });
}
