const maxContentLength = 8_000
const maxToolSummaryLength = 512

export const limitTranscriptContent = (content: string): string =>
  content.length <= maxContentLength
    ? content
    : `${content.slice(0, maxContentLength)}\n…`

export const boundedTranscriptValue = (value: unknown): unknown => {
  let remaining = maxContentLength
  let remainingNodes = 512
  const seen = new WeakSet<object>()
  const visit = (current: unknown, depth: number): unknown => {
    if (remainingNodes <= 0) return "[…]"
    remainingNodes -= 1
    if (depth >= 20) return "[max depth]"
    if (typeof current === "string") {
      const take = Math.max(0, Math.min(current.length, remaining))
      remaining -= take
      return take === current.length
        ? current
        : `${current.slice(0, take)}\n…`
    }
    if (
      current === null ||
      current === undefined ||
      typeof current === "number" ||
      typeof current === "boolean"
    ) {
      remaining -= String(current).length
      return current
    }
    if (typeof current !== "object") return String(current)
    if (seen.has(current)) return "[circular]"
    seen.add(current)

    if (Array.isArray(current)) {
      const result: Array<unknown> = []
      for (const item of current) {
        if (remaining <= 0 || remainingNodes <= 0) {
          result.push("[…]")
          break
        }
        result.push(visit(item, depth + 1))
      }
      return result
    }

    const result: Record<string, unknown> = {}
    for (const key in current) {
      if (!Object.hasOwn(current, key)) continue
      if (remaining <= 0 || remainingNodes <= 0) {
        result.truncated = true
        break
      }
      remaining -= key.length
      result[key] = visit(
        (current as Record<string, unknown>)[key],
        depth + 1,
      )
    }
    return result
  }
  return visit(value, 0)
}

export const asTranscriptRecord = (
  value: unknown,
): Record<string, unknown> | undefined =>
  typeof value === "object" && value !== null
    ? (value as Record<string, unknown>)
    : undefined

export const transcriptMessageRole = (
  message: unknown,
): string | undefined => {
  const record = asTranscriptRecord(message)
  return typeof record?.role === "string" ? record.role : undefined
}

export const transcriptTextParts = (
  content: unknown,
): ReadonlyArray<string> => {
  if (typeof content === "string") return [content]
  if (!Array.isArray(content)) return []

  return content.flatMap((part) => {
    const record = asTranscriptRecord(part)
    if (record?.type === "text" && typeof record.text === "string") {
      return [record.text]
    }
    return []
  })
}

export interface AssistantContentParts {
  readonly text: string
  readonly thinking: string
  readonly thinkingRedacted: boolean
}

export const assistantContentParts = (
  message: unknown,
): AssistantContentParts => {
  const record = asTranscriptRecord(message)
  const content = Array.isArray(record?.content) ? record.content : []
  let thinkingRedacted = false
  const thinking = content.flatMap((part) => {
    const value = asTranscriptRecord(part)
    if (value?.type !== "thinking") return []
    if (value.redacted === true) {
      thinkingRedacted = true
      return []
    }
    return typeof value.thinking === "string" ? [value.thinking] : []
  })

  return {
    text: limitTranscriptContent(transcriptTextParts(content).join("")),
    thinking: limitTranscriptContent(thinking.join("")),
    thinkingRedacted,
  }
}

export const assistantErrorMessage = (
  message: unknown,
): string | undefined => {
  const record = asTranscriptRecord(message)
  return typeof record?.errorMessage === "string"
    ? record.errorMessage
    : undefined
}

const formatUnknown = (value: unknown): string => {
  if (typeof value === "string") return limitTranscriptContent(value)

  const record = asTranscriptRecord(value)
  const content = transcriptTextParts(record?.content).join("\n")
  if (content.length > 0) return limitTranscriptContent(content)

  try {
    return limitTranscriptContent(
      JSON.stringify(value, null, 2) ?? String(value),
    )
  } catch {
    return limitTranscriptContent(String(value))
  }
}

export const formatToolTranscriptSummary = (value: unknown): string => {
  const content = formatUnknown(value)
  return content.length <= maxToolSummaryLength
    ? content
    : `${content.slice(0, maxToolSummaryLength)}\n…`
}

export const transcriptToolCalls = (
  message: unknown,
): ReadonlyArray<{
  readonly id: string
  readonly name: string
  readonly arguments: unknown
}> => {
  const record = asTranscriptRecord(message)
  const content = Array.isArray(record?.content) ? record.content : []
  return content.flatMap((part) => {
    const value = asTranscriptRecord(part)
    if (
      value?.type !== "toolCall" ||
      typeof value.id !== "string" ||
      typeof value.name !== "string"
    ) {
      return []
    }
    return [{ id: value.id, name: value.name, arguments: value.arguments }]
  })
}
