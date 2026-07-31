import type { AgentSessionEvent } from "@earendil-works/pi-coding-agent"
import { expect, test } from "bun:test"
import {
  emptyLiveUsage,
  reduceLiveUsage,
  sessionStatsWithLiveUsage,
} from "../src/services/live-usage.ts"

const event = (value: unknown): AgentSessionEvent =>
  value as AgentSessionEvent

const assistant = (total: number, cost: number) => ({
  role: "assistant",
  content: [],
  usage: {
    input: total - 2,
    output: 2,
    cacheRead: 0,
    cacheWrite: 0,
    totalTokens: total,
    cost: { total: cost },
  },
})

test("replaces partial usage and adds each finished assistant once", () => {
  const firstPartial = reduceLiveUsage(
    emptyLiveUsage,
    event({ type: "message_start", message: assistant(3, 0.01) }),
  )
  const latestPartial = reduceLiveUsage(
    firstPartial,
    event({ type: "message_update", message: assistant(8, 0.03) }),
  )
  const firstDone = reduceLiveUsage(
    latestPartial,
    event({ type: "message_end", message: assistant(10, 0.04) }),
  )
  const secondDone = reduceLiveUsage(
    firstDone,
    event({ type: "message_end", message: assistant(5, 0.02) }),
  )

  expect(latestPartial.current.total).toBe(8)
  expect(firstDone.completed.total).toBe(10)
  expect(firstDone.current.total).toBe(0)
  expect(secondDone.completed.total).toBe(15)
})

test("adds unsettled usage to the last saved session totals", () => {
  const live = reduceLiveUsage(
    emptyLiveUsage,
    event({ type: "message_update", message: assistant(8, 0.03) }),
  )
  const stats = sessionStatsWithLiveUsage(
    {
      sessionFile: undefined,
      sessionId: "session-1",
      userMessages: 1,
      assistantMessages: 1,
      toolCalls: 0,
      toolResults: 0,
      totalMessages: 2,
      tokens: {
        input: 10,
        output: 2,
        cacheRead: 0,
        cacheWrite: 0,
        total: 12,
      },
      cost: 0.1,
    },
    live,
  )

  expect(stats.tokens.total).toBe(20)
  expect(stats.cost).toBeCloseTo(0.13)
})

test("keeps completed retry usage across later agent starts", () => {
  const firstDone = reduceLiveUsage(
    emptyLiveUsage,
    event({ type: "message_end", message: assistant(10, 0.04) }),
  )
  const retryStarted = reduceLiveUsage(
    firstDone,
    event({ type: "agent_start" }),
  )
  const retryPartial = reduceLiveUsage(
    retryStarted,
    event({ type: "message_update", message: assistant(6, 0.02) }),
  )

  expect(retryPartial.completed.total).toBe(10)
  expect(retryPartial.current.total).toBe(6)
  expect(
    reduceLiveUsage(retryPartial, event({ type: "agent_settled" })),
  ).toEqual(emptyLiveUsage)
})
