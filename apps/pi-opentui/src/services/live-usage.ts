import type {
  AgentSessionEvent,
  SessionStats,
} from "@earendil-works/pi-coding-agent"

export interface UsageTotals {
  readonly input: number
  readonly output: number
  readonly cacheRead: number
  readonly cacheWrite: number
  readonly total: number
  readonly cost: number
}

export interface LiveUsage {
  readonly completed: UsageTotals
  readonly current: UsageTotals
}

export const emptyUsageTotals: UsageTotals = {
  input: 0,
  output: 0,
  cacheRead: 0,
  cacheWrite: 0,
  total: 0,
  cost: 0,
}

export const emptyLiveUsage: LiveUsage = {
  completed: emptyUsageTotals,
  current: emptyUsageTotals,
}

const asRecord = (value: unknown): Record<string, unknown> | undefined =>
  typeof value === "object" && value !== null
    ? (value as Record<string, unknown>)
    : undefined

const numberValue = (value: unknown): number =>
  typeof value === "number" && Number.isFinite(value) ? value : 0

export const usageFromAssistantMessage = (
  message: unknown,
): UsageTotals | undefined => {
  const record = asRecord(message)
  if (record?.role !== "assistant") return undefined

  const usage = asRecord(record.usage)
  const cost = asRecord(usage?.cost)
  if (usage === undefined) return undefined

  const input = numberValue(usage.input)
  const output = numberValue(usage.output)
  const cacheRead = numberValue(usage.cacheRead)
  const cacheWrite = numberValue(usage.cacheWrite)
  return {
    input,
    output,
    cacheRead,
    cacheWrite,
    total: numberValue(usage.totalTokens) ||
      input + output + cacheRead + cacheWrite,
    cost: numberValue(cost?.total),
  }
}

const addUsage = (left: UsageTotals, right: UsageTotals): UsageTotals => ({
  input: left.input + right.input,
  output: left.output + right.output,
  cacheRead: left.cacheRead + right.cacheRead,
  cacheWrite: left.cacheWrite + right.cacheWrite,
  total: left.total + right.total,
  cost: left.cost + right.cost,
})

export const reduceLiveUsage = (
  live: LiveUsage,
  event: AgentSessionEvent,
): LiveUsage => {
  switch (event.type) {
    case "agent_settled":
      return emptyLiveUsage

    case "agent_start":
      return { ...live, current: emptyUsageTotals }

    case "message_start":
    case "message_update": {
      const current = usageFromAssistantMessage(event.message)
      return current === undefined ? live : { ...live, current }
    }

    case "message_end": {
      const finalUsage = usageFromAssistantMessage(event.message)
      if (finalUsage === undefined) return live
      return {
        completed: addUsage(live.completed, finalUsage),
        current: emptyUsageTotals,
      }
    }

    default:
      return live
  }
}

export const sessionStatsWithLiveUsage = (
  stats: SessionStats,
  live: LiveUsage,
): SessionStats => {
  const pending = addUsage(live.completed, live.current)
  return {
    ...stats,
    tokens: {
      input: stats.tokens.input + pending.input,
      output: stats.tokens.output + pending.output,
      cacheRead: stats.tokens.cacheRead + pending.cacheRead,
      cacheWrite: stats.tokens.cacheWrite + pending.cacheWrite,
      total: stats.tokens.total + pending.total,
    },
    cost: stats.cost + pending.cost,
  }
}
