import type { SessionStats } from "@earendil-works/pi-coding-agent"

interface UsageParts {
  readonly input: number
  readonly output: number
  readonly cacheRead: number
  readonly cacheWrite: number
  readonly cost: number
}

const record = (value: unknown): Record<string, unknown> | undefined =>
  typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined

const nonNegative = (value: unknown): number =>
  typeof value === "number" && Number.isFinite(value) && value >= 0
    ? value
    : 0

const remoteUsage = (entry: unknown): UsageParts | undefined => {
  const value = record(entry)
  if (value?.type !== "compaction") return undefined
  const details = record(value.details)
  const remoteCompaction = record(details?.remoteCompaction)
  const usage = record(remoteCompaction?.usage)
  if (usage === undefined) return undefined
  const cost = record(usage.cost)
  return {
    input: nonNegative(usage.input),
    output: nonNegative(usage.output),
    cacheRead: nonNegative(usage.cacheRead),
    cacheWrite: nonNegative(usage.cacheWrite),
    cost: nonNegative(cost?.total),
  }
}

export const sessionStatsWithCompactionUsage = (
  stats: SessionStats,
  entries: ReadonlyArray<unknown>,
): SessionStats => {
  const added = entries.reduce<UsageParts>(
    (total, entry) => {
      const usage = remoteUsage(entry)
      return usage === undefined
        ? total
        : {
            input: total.input + usage.input,
            output: total.output + usage.output,
            cacheRead: total.cacheRead + usage.cacheRead,
            cacheWrite: total.cacheWrite + usage.cacheWrite,
            cost: total.cost + usage.cost,
          }
    },
    { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0 },
  )
  const tokens = {
    input: stats.tokens.input + added.input,
    output: stats.tokens.output + added.output,
    cacheRead: stats.tokens.cacheRead + added.cacheRead,
    cacheWrite: stats.tokens.cacheWrite + added.cacheWrite,
  }
  return {
    ...stats,
    tokens: {
      ...tokens,
      total:
        tokens.input +
        tokens.output +
        tokens.cacheRead +
        tokens.cacheWrite,
    },
    cost: stats.cost + added.cost,
  }
}
