import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";

const Params = Type.Object({
  question: Type.String({ description: "A short question for the user" }),
  options: Type.Optional(Type.Array(Type.String(), { description: "Optional choices" })),
});

export default function (pi: ExtensionAPI) {
  pi.registerTool({
    name: "request_user_input",
    label: "Ask user",
    description: "Ask the user one clear question when their input is needed to continue.",
    parameters: Params,
    executionMode: "sequential",
    async execute(toolCallId, params, _signal, _onUpdate, ctx) {
      if (!ctx.hasUI) {
        return { content: [{ type: "text", text: "User input is unavailable in this mode" }], isError: true };
      }

      pi.events.emit("approval:requested", {
        kind: "user-input",
        title: "Pi needs input",
        summary: params.question,
        toolName: "request_user_input",
        toolCallId,
        sessionId: ctx.sessionManager.getSessionId(),
        cwd: ctx.cwd,
      });

      const options = params.options?.filter((option) => option.trim()) ?? [];
      const answer = options.length
        ? await ctx.ui.select(params.question, options)
        : await ctx.ui.input(params.question, "Type your answer");
      pi.events.emit("approval:resolved", {
        kind: "user-input",
        toolName: "request_user_input",
        toolCallId,
        decision: answer === undefined ? "denied" : "allowed",
      });

      return {
        content: [{ type: "text", text: answer === undefined ? "User cancelled" : `User answered: ${answer}` }],
        details: { answered: answer !== undefined },
      };
    },
  });
}
