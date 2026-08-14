import { expect, test } from "bun:test"
import { createAssistantMessageEventStream } from "@earendil-works/pi-ai"
import { processResponsesStream } from "../node_modules/@earendil-works/pi-ai/dist/api/openai-responses-shared.js"

test("the Pi AI patch exposes completed native output items", async () => {
  const item = { type: "compaction", encrypted_content: "opaque" }
  const events = (async function* () {
    yield {
      type: "response.output_item.done",
      output_index: 0,
      item,
    }
  })()
  const output = {
    role: "assistant",
    content: [],
    api: "openai-codex-responses",
    provider: "openai-codex",
    model: "gpt-5.6-sol",
    usage: {
      input: 0,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      totalTokens: 0,
      cost: {
        input: 0,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        total: 0,
      },
    },
    stopReason: "stop",
    timestamp: 0,
  }
  const seen: unknown[] = []

  await expect(
    processResponsesStream(
      events as never,
      output as never,
      createAssistantMessageEventStream(),
      { id: "gpt-5.6-sol" } as never,
      {
        onOutputItemDone: (value) => {
          seen.push(value)
        },
      },
    ),
  ).rejects.toThrow(/terminal response event/)
  expect(seen).toEqual([item])
})
