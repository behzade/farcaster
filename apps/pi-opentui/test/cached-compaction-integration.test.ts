import { afterEach, expect, test } from "bun:test"
import { openAICodexResponsesApi } from "@earendil-works/pi-ai/compat"
import {
  closeOpenAICodexWebSocketSessions,
  resetOpenAICodexWebSocketDebugStats,
} from "../node_modules/@earendil-works/pi-ai/dist/api/openai-codex-responses.js"

const sentBodies: Array<Record<string, unknown>> = []
const priorWebSocket = globalThis.WebSocket

class FakeWebSocket {
  static readonly OPEN = 1
  readonly readyState = FakeWebSocket.OPEN
  private readonly listeners = new Map<string, Set<(event: unknown) => void>>()

  constructor(_url: string | URL, _options?: unknown) {
    queueMicrotask(() => this.emit("open", {}))
  }

  addEventListener(type: string, listener: (event: unknown) => void): void {
    const listeners = this.listeners.get(type) ?? new Set()
    listeners.add(listener)
    this.listeners.set(type, listeners)
  }

  removeEventListener(type: string, listener: (event: unknown) => void): void {
    this.listeners.get(type)?.delete(listener)
  }

  send(raw: string): void {
    const body = JSON.parse(raw) as Record<string, unknown>
    sentBodies.push(body)
    queueMicrotask(() => {
      if (sentBodies.length === 1) {
        this.message({
          type: "response.output_item.done",
          output_index: 0,
          item: {
            type: "message",
            id: "msg_1",
            role: "assistant",
            status: "completed",
            content: [
              {
                type: "output_text",
                text: "first answer",
                annotations: [],
                logprobs: [],
              },
            ],
          },
        })
        this.completed("resp_1", 20, 0)
        return
      }
      this.message({
        type: "response.output_item.done",
        output_index: 0,
        item: {
          type: "compaction",
          id: "compact_1",
          encrypted_content: "opaque",
        },
      })
      this.completed("resp_compact", 925, 900)
    })
  }

  close(): void {}

  private message(value: unknown): void {
    this.emit("message", { data: JSON.stringify(value) })
  }

  private completed(id: string, inputTokens: number, cachedTokens: number): void {
    this.message({
      type: "response.completed",
      response: {
        id,
        status: "completed",
        output: [],
        usage: {
          input_tokens: inputTokens,
          input_tokens_details: { cached_tokens: cachedTokens },
          output_tokens: 5,
          output_tokens_details: { reasoning_tokens: 0 },
          total_tokens: inputTokens + 5,
        },
      },
    })
  }

  private emit(type: string, event: unknown): void {
    for (const listener of this.listeners.get(type) ?? []) listener(event)
  }
}

const model = {
  id: "gpt-5.6-sol",
  name: "GPT 5.6 Sol",
  api: "openai-codex-responses",
  provider: "openai-codex",
  baseUrl: "https://chatgpt.com/backend-api",
  reasoning: true,
  input: ["text"],
  contextWindow: 200_000,
  maxTokens: 32_000,
  cost: { input: 1, output: 1, cacheRead: 1, cacheWrite: 1 },
} as const

const token = `header.${Buffer.from(
  JSON.stringify({
    "https://api.openai.com/auth": { chatgpt_account_id: "account-1" },
  }),
).toString("base64url")}.signature`

afterEach(() => {
  closeOpenAICodexWebSocketSessions("cache-test")
  resetOpenAICodexWebSocketDebugStats("cache-test")
  globalThis.WebSocket = priorWebSocket
  sentBodies.length = 0
})

test("Codex compaction reuses the active response and sends only the trigger", async () => {
  globalThis.WebSocket = FakeWebSocket as never
  const provider = openAICodexResponsesApi()
  const userMessage = {
    role: "user",
    content: [{ type: "text", text: "hello" }],
    timestamp: 1,
  } as const
  const context = {
    systemPrompt: "system",
    messages: [userMessage],
  }
  const commonOptions = {
    apiKey: token,
    sessionId: "cache-test",
    transport: "websocket-cached",
    reasoningEffort: "high",
    textVerbosity: "low",
  } as const

  let firstMessage: unknown
  for await (const event of provider.stream(model as never, context as never, commonOptions as never)) {
    if (event.type === "done") firstMessage = event.message
  }

  let compactionItem: unknown
  let compactionUsage: { cacheRead?: number } | undefined
  for await (const event of provider.stream(
    model as never,
    { ...context, messages: [userMessage, firstMessage] } as never,
    {
      ...commonOptions,
      onPayload: (payload: unknown) => {
        const body = payload as Record<string, unknown>
        return {
          ...body,
          input: [
            ...((body.input as unknown[]) ?? []),
            { type: "compaction_trigger" },
          ],
        }
      },
      onOutputItemDone: (item: unknown) => {
        compactionItem = item
      },
    } as never,
  )) {
    if (event.type === "done") compactionUsage = event.message.usage
  }

  expect(sentBodies).toHaveLength(2)
  expect(sentBodies[1]?.previous_response_id).toBe("resp_1")
  expect(sentBodies[1]?.input).toEqual([{ type: "compaction_trigger" }])
  expect(compactionUsage?.cacheRead).toBe(900)
  expect(compactionItem).toMatchObject({
    type: "compaction",
    encrypted_content: "opaque",
  })
})
