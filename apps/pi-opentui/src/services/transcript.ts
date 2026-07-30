import type {
  AgentSessionEvent,
} from "@earendil-works/pi-coding-agent"

export type TranscriptRowKind =
  | "user"
  | "assistant"
  | "tool"
  | "notice"
  | "error"

export interface TranscriptRow {
  readonly id: string
  readonly kind: TranscriptRowKind
  readonly title: string
  readonly content: string
  readonly pending: boolean
  readonly isError: boolean
}

export interface TranscriptModel {
  readonly rows: ReadonlyArray<TranscriptRow>
  readonly activeAssistantId: string | undefined
  readonly nextRowId: number
}

export const emptyTranscript: TranscriptModel = {
  rows: [],
  activeAssistantId: undefined,
  nextRowId: 1,
}

const maxRows = 200
const maxContentLength = 8_000

const limitContent = (content: string): string =>
  content.length <= maxContentLength
    ? content
    : `${content.slice(0, maxContentLength)}\n…`

const appendRow = (
  model: TranscriptModel,
  row: TranscriptRow,
): TranscriptModel => ({
  ...model,
  rows: [...model.rows, row].slice(-maxRows),
  nextRowId: model.nextRowId + 1,
})

const updateRow = (
  model: TranscriptModel,
  id: string,
  update: (row: TranscriptRow) => TranscriptRow,
): TranscriptModel => ({
  ...model,
  rows: model.rows.map((row) => (row.id === id ? update(row) : row)),
})

const asRecord = (value: unknown): Record<string, unknown> | undefined =>
  typeof value === "object" && value !== null
    ? (value as Record<string, unknown>)
    : undefined

const messageRole = (message: unknown): string | undefined => {
  const record = asRecord(message)
  return typeof record?.role === "string" ? record.role : undefined
}

const textParts = (content: unknown): ReadonlyArray<string> => {
  if (typeof content === "string") return [content]
  if (!Array.isArray(content)) return []

  return content.flatMap((part) => {
    const record = asRecord(part)
    if (record?.type === "text" && typeof record.text === "string") {
      return [record.text]
    }
    return []
  })
}

const assistantText = (message: unknown): string => {
  const record = asRecord(message)
  const text = textParts(record?.content).join("")
  if (text.length > 0) return limitContent(text)

  const content = Array.isArray(record?.content) ? record.content : []
  const isThinking = content.some(
    (part) => asRecord(part)?.type === "thinking",
  )
  return isThinking ? "Thinking…" : ""
}

const errorMessage = (message: unknown): string | undefined => {
  const record = asRecord(message)
  return typeof record?.errorMessage === "string"
    ? record.errorMessage
    : undefined
}

const formatUnknown = (value: unknown): string => {
  if (typeof value === "string") return limitContent(value)

  const record = asRecord(value)
  const content = textParts(record?.content).join("\n")
  if (content.length > 0) return limitContent(content)

  try {
    return limitContent(JSON.stringify(value, null, 2) ?? String(value))
  } catch {
    return limitContent(String(value))
  }
}

export const appendUserPrompt = (
  model: TranscriptModel,
  prompt: string,
): TranscriptModel =>
  appendRow(model, {
    id: `row-${model.nextRowId}`,
    kind: "user",
    title: "you",
    content: limitContent(prompt),
    pending: false,
    isError: false,
  })

export const appendTranscriptError = (
  model: TranscriptModel,
  message: string,
): TranscriptModel =>
  appendRow(model, {
    id: `row-${model.nextRowId}`,
    kind: "error",
    title: "error",
    content: limitContent(message),
    pending: false,
    isError: true,
  })

export const appendTranscriptNotice = (
  model: TranscriptModel,
  message: string,
  isError = false,
): TranscriptModel =>
  appendRow(model, {
    id: `row-${model.nextRowId}`,
    kind: isError ? "error" : "notice",
    title: isError ? "extension error" : "notice",
    content: limitContent(message),
    pending: false,
    isError,
  })

export const reduceTranscriptEvent = (
  model: TranscriptModel,
  event: AgentSessionEvent,
): TranscriptModel => {
  switch (event.type) {
    case "message_start": {
      if (messageRole(event.message) !== "assistant") return model

      const id = `row-${model.nextRowId}`
      return {
        ...appendRow(model, {
          id,
          kind: "assistant",
          title: "pi",
          content: assistantText(event.message),
          pending: true,
          isError: false,
        }),
        activeAssistantId: id,
      }
    }

    case "message_update": {
      if (messageRole(event.message) !== "assistant") return model

      if (model.activeAssistantId === undefined) {
        return reduceTranscriptEvent(model, {
          type: "message_start",
          message: event.message,
        })
      }
      return updateRow(model, model.activeAssistantId, (row) => ({
        ...row,
        content: assistantText(event.message),
      }))
    }

    case "message_end": {
      if (
        messageRole(event.message) !== "assistant" ||
        model.activeAssistantId === undefined
      ) {
        return model
      }

      const fault = errorMessage(event.message)
      const content = [assistantText(event.message), fault]
        .filter((part): part is string => part !== undefined && part.length > 0)
        .join("\n")
      return {
        ...updateRow(model, model.activeAssistantId, (row) => ({
          ...row,
          content,
          pending: false,
          isError: fault !== undefined,
        })),
        activeAssistantId: undefined,
      }
    }

    case "tool_execution_start":
      return appendRow(model, {
        id: `tool-${event.toolCallId}`,
        kind: "tool",
        title: event.toolName,
        content: formatUnknown(event.args),
        pending: true,
        isError: false,
      })

    case "tool_execution_update":
      return updateRow(model, `tool-${event.toolCallId}`, (row) => ({
        ...row,
        content: formatUnknown(event.partialResult),
      }))

    case "tool_execution_end":
      return updateRow(model, `tool-${event.toolCallId}`, (row) => ({
        ...row,
        content: formatUnknown(event.result),
        pending: false,
        isError: event.isError,
      }))

    case "compaction_start":
      return appendTranscriptNotice(
        model,
        `Compaction started (${event.reason})`,
      )

    case "compaction_end":
      return appendTranscriptNotice(
        model,
        event.errorMessage ??
          (event.aborted
            ? "Compaction stopped"
            : "Compaction finished"),
        event.errorMessage !== undefined,
      )

    case "auto_retry_start":
      return appendTranscriptNotice(
        model,
        `Retry ${event.attempt}/${event.maxAttempts} in ${event.delayMs}ms: ${event.errorMessage}`,
      )

    case "auto_retry_end":
      return appendTranscriptNotice(
        model,
        event.success
          ? `Retry ${event.attempt} succeeded`
          : (event.finalError ?? `Retry ${event.attempt} failed`),
        !event.success,
      )

    default:
      return model
  }
}
