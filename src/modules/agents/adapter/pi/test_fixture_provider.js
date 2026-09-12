import { appendFileSync } from "node:fs";

export default function fixtureProvider(pi) {
  const logPath = process.env.FARCASTER_PI_FIXTURE_LOG;
  pi.registerProvider("farcaster-fixture", {
    api: "farcaster-fixture",
    baseUrl: "http://127.0.0.1.invalid",
    apiKey: "fixture",
    models: [{
      id: "fixture",
      name: "Farcaster fixture",
      reasoning: false,
      input: ["text", "image"],
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
      contextWindow: 8192,
      maxTokens: 1024,
    }],
    streamSimple(model, context, options) {
      const userTexts = context.messages
        .filter((message) => message.role === "user")
        .map((message) => typeof message.content === "string"
          ? message.content
          : (message.content ?? []).filter((part) => part.type === "text").map((part) => part.text).join(""));
      const text = userTexts.at(-1) ?? "";
      appendFileSync(logPath, `${JSON.stringify(userTexts)}\n`);

      const output = {
        role: "assistant",
        content: [{ type: "text", text: `done: ${text}` }],
        api: model.api,
        provider: model.provider,
        model: model.id,
        usage: {
          input: 0,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          totalTokens: 0,
          cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
        },
        stopReason: "stop",
        timestamp: Date.now(),
      };
      let resolveResult;
      const result = new Promise((resolve) => { resolveResult = resolve; });
      return {
        async *[Symbol.asyncIterator]() {
          if (text.startsWith("hold")) {
            await new Promise((resolve) => {
              if (options?.signal?.aborted) resolve();
              else options?.signal?.addEventListener("abort", resolve, { once: true });
            });
            output.stopReason = "aborted";
            output.errorMessage = "fixture request aborted";
            resolveResult(output);
            yield { type: "error", reason: "aborted", error: output };
          } else {
            resolveResult(output);
            yield { type: "done", reason: "stop", message: output };
          }
        },
        result() { return result; },
      };
    },
  });
}
