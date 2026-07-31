import { describe, expect, test } from "bun:test"
import type { SessionStats } from "@earendil-works/pi-coding-agent"
import { sessionStatsWithCompactionUsage } from "../src/services/compaction-usage.ts"

const stats: SessionStats = {
  sessionFile: undefined,
  sessionId: "session-1",
  userMessages: 1,
  assistantMessages: 1,
  toolCalls: 0,
  toolResults: 0,
  totalMessages: 2,
  tokens: {
    input: 100,
    output: 20,
    cacheRead: 300,
    cacheWrite: 0,
    total: 420,
  },
  cost: 0.25,
}

describe("sessionStatsWithCompactionUsage", () => {
  test("adds persisted remote compaction tokens and cost", () => {
    const result = sessionStatsWithCompactionUsage(stats, [
      {
        type: "compaction",
        details: {
          remoteCompaction: {
            usage: {
              input: 5,
              output: 7,
              cacheRead: 900,
              cacheWrite: 3,
              totalTokens: 915,
              cost: { total: 0.1 },
            },
          },
        },
      },
    ])

    expect(result.tokens).toEqual({
      input: 105,
      output: 27,
      cacheRead: 1200,
      cacheWrite: 3,
      total: 1335,
    })
    expect(result.cost).toBeCloseTo(0.35)
  })

  test("ignores local, malformed, and negative usage", () => {
    const result = sessionStatsWithCompactionUsage(stats, [
      { type: "compaction", details: {} },
      {
        type: "compaction",
        details: {
          remoteCompaction: {
            usage: {
              input: -5,
              output: Number.NaN,
              cacheRead: "900",
              cacheWrite: 0,
              cost: { total: -1 },
            },
          },
        },
      },
      { type: "message", details: { remoteCompaction: { usage: {} } } },
    ])

    expect(result).toEqual(stats)
  })
})
