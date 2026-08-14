import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { Effect } from "effect";
import {
  executeContinuationCompaction,
  responsesCompactionStreamLayer,
  type ContinuationCompactionStream,
} from "../src/continuation-compaction.ts";
import {
  mergeProviderHeaders,
  resolveProviderHeaders,
  withRemoteCompactionV2Feature,
} from "../src/provider-headers.ts";

const usage = {
  input: 12,
  output: 4,
  cacheRead: 900,
  cacheWrite: 0,
  totalTokens: 916,
  cost: {
    input: 0.01,
    output: 0.02,
    cacheRead: 0.03,
    cacheWrite: 0,
    total: 0.06,
  },
};

function runWith(stream: ContinuationCompactionStream, params: {
  explicitPromptInput?: Array<Record<string, unknown>>;
  requestShape?: Record<string, unknown>;
} = {}) {
  return Effect.runPromise(executeContinuationCompaction({
    model: { id: "gpt-5.6-sol" },
    context: { systemPrompt: "system", messages: [] },
    streamOptions: { transport: "websocket-cached" },
    ...params,
  }).pipe(Effect.provide(responsesCompactionStreamLayer(stream))));
}

describe("executeContinuationCompaction", () => {
  it("adds only the trigger and keeps cache usage from the normal stream", async () => {
    let sentBody: Record<string, unknown> | undefined;
    const stream: ContinuationCompactionStream = (_model, _context, options) => (async function* () {
      sentBody = (options.onPayload as (body: unknown) => Record<string, unknown>)({
        model: "gpt-5.6-sol",
        input: [{ type: "message", role: "user", content: [{ type: "input_text", text: "hi" }] }],
        instructions: "generated",
        tools: [{ type: "function", name: "old" }],
        text: { verbosity: "low" },
      });
      await (options.onOutputItemDone as (item: unknown) => void)({
        type: "compaction",
        encrypted_content: "opaque",
      });
      yield {
        type: "done",
        reason: "stop",
        message: {
          stopReason: "stop",
          responseId: "resp_compact",
          usage,
        },
      };
    })();

    const result = await runWith(stream, {
      requestShape: {
        instructions: "observed",
        tools: [{ type: "function", name: "read" }],
        parallelToolCalls: true,
        toolChoice: "auto",
        reasoning: { effort: "high", summary: "auto" },
        text: { verbosity: "medium" },
      },
    });

    assert.deepEqual(sentBody?.input, [
      { type: "message", role: "user", content: [{ type: "input_text", text: "hi" }] },
      { type: "compaction_trigger" },
    ]);
    assert.equal(sentBody?.instructions, "observed");
    assert.deepEqual(sentBody?.tools, [{ type: "function", name: "read" }]);
    assert.deepEqual(sentBody?.reasoning, { effort: "high", summary: "auto" });
    assert.deepEqual(result.usage, usage);
    assert.equal(result.compactionItem.type, "compaction");
  });

  it("uses persisted compacted history instead of regenerated branch input", async () => {
    let sentBody: Record<string, unknown> | undefined;
    const stream: ContinuationCompactionStream = (_model, _context, options) => (async function* () {
      sentBody = (options.onPayload as (body: unknown) => Record<string, unknown>)({
        input: [{ type: "message", role: "user", content: [] }],
      });
      await (options.onOutputItemDone as (item: unknown) => void)({
        type: "compaction",
        encrypted_content: "next",
      });
      yield {
        type: "done",
        reason: "stop",
        message: { stopReason: "stop", responseId: "resp_2", usage },
      };
    })();

    const explicitPromptInput = [
      { type: "message", role: "user", content: [{ type: "input_text", text: "kept" }] },
      { type: "compaction", encrypted_content: "prior" },
    ];
    const result = await runWith(stream, { explicitPromptInput });

    assert.deepEqual(sentBody?.input, [...explicitPromptInput, { type: "compaction_trigger" }]);
    assert.deepEqual(result.promptInput, explicitPromptInput);
  });

  it("fails if the provider does not expose one compaction item", async () => {
    const stream: ContinuationCompactionStream = (_model, _context, options) => (async function* () {
      (options.onPayload as (body: unknown) => unknown)({ input: [] });
      yield {
        type: "done",
        reason: "stop",
        message: { stopReason: "stop", responseId: "resp_bad", usage },
      };
    })();

    await assert.rejects(runWith(stream), /expected one compaction output item/);
  });
});

describe("provider headers", () => {
  it("keeps null deletion markers until the raw request boundary", () => {
    const merged = mergeProviderHeaders(
      { Authorization: "Bearer default", "X-Keep": "yes" },
      { authorization: null, "x-extra": "value" },
    );

    assert.deepEqual(merged, {
      authorization: null,
      "X-Keep": "yes",
      "x-extra": "value",
    });
    assert.deepEqual(resolveProviderHeaders(merged), {
      "X-Keep": "yes",
      "x-extra": "value",
    });
  });

  it("handles a null feature header and still requests remote compaction v2", () => {
    const headers = withRemoteCompactionV2Feature({
      "X-Codex-Beta-Features": null,
      "X-Remove": null,
    });

    assert.deepEqual(headers, {
      "X-Remove": null,
      "x-codex-beta-features": "remote_compaction_v2",
    });
  });
});
