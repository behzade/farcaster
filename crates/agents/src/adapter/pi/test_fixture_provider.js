import { appendFileSync, existsSync } from "node:fs";
import bridge from "./farcaster.js";

export default async function fixtureProvider(pi) {
  const nativeFixture = existsSync("fixture-native-tools");
  if (nativeFixture) {
    const url = "http://farcaster-fixture.invalid/mcp";
    globalThis.fetch = async (target, options) => {
      if (target !== url) throw new Error(`Unexpected fixture fetch: ${target}`);
      const request = JSON.parse(options.body);
      let result = {};
      if (request.method === "tools/list") {
        result = {tools: [{
          name: "fixture_result",
          inputSchema: {type: "object", properties: {kind: {type: "string", enum: ["ok", "error", "review"]}}, required: ["kind"]},
          outputSchema: {type: "object"},
        }]};
      } else if (request.method === "tools/call") {
        const kind = request.params.arguments.kind;
        const structuredContent = {
          ok: {value: 42}, error: {errorCode: "fixture-error"}, review: {farcaster_review: {version: 1}},
        }[kind];
        if (!structuredContent || request.params.name !== "fixture_result") throw new Error("Unexpected fixture tool call");
        result = {
          content: [{type: "text", text: kind === "review" ? JSON.stringify(structuredContent) : "display text"}],
          structuredContent, isError: kind === "error",
        };
      } else if (!["initialize", "notifications/initialized"].includes(request.method)) {
        throw new Error(`Unexpected fixture method: ${request.method}`);
      }
      return new Response(JSON.stringify({jsonrpc: "2.0", id: request.id, result}), {
        headers: {"content-type": "application/json"},
      });
    };
    const previousUrl = process.env.FARCASTER_MCP_URL;
    const previousToken = process.env.FARCASTER_MCP_CALLER;
    process.env.FARCASTER_MCP_URL = url;
    process.env.FARCASTER_MCP_CALLER = "fixture";
    try { await bridge(pi); }
    finally {
      if (previousUrl === undefined) delete process.env.FARCASTER_MCP_URL;
      else process.env.FARCASTER_MCP_URL = previousUrl;
      if (previousToken === undefined) delete process.env.FARCASTER_MCP_CALLER;
      else process.env.FARCASTER_MCP_CALLER = previousToken;
    }
    pi.on("session_start", () => pi.setActiveTools([...pi.getActiveTools(), "codemode"]));
  }
  const logPath = process.env.FARCASTER_PI_FIXTURE_LOG;
  pi.registerProvider("farcaster-fixture", {
    api: "farcaster-fixture",
    baseUrl: "http://127.0.0.1.invalid",
    apiKey: "fixture",
    models: ["fixture", "fixture-child", "fixture-other"].map((id) => ({
      id,
      name: "Farcaster fixture",
      reasoning: id !== "fixture",
      input: ["text", "image"],
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
      contextWindow: 8192,
      maxTokens: 1024,
    })),
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
          if (nativeFixture && text === "compose fixture" && context.messages.at(-1)?.role !== "toolResult") {
            output.content = [{type: "toolCall", id: "fixture-compose", name: "codemode", arguments: {
              code: 'const ok = await tools.farcaster_fixture_result({kind: "ok"}); const error = await tools.farcaster_fixture_result({kind: "error"}); const review = await tools.farcaster_fixture_result({kind: "review"}); text({ok: ok.value, error: error.errorCode, review: review.farcaster_review.version});',
            }}];
            output.stopReason = "toolUse";
            resolveResult(output);
            yield {type: "done", reason: "toolUse", message: output};
          } else if (text === "hold tool" && !options?.signal?.aborted) {
            output.content = [{ type: "toolCall", id: "fixture-tool", name: "bash", arguments: { command: "printf started > fixture-tool-started; sleep 30" } }];
            output.stopReason = "toolUse";
            resolveResult(output);
            yield { type: "done", reason: "toolUse", message: output };
          } else if (options?.signal?.aborted || text.startsWith("hold")) {
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
