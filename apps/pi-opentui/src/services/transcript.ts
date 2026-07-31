import type { AgentSessionEvent } from "@earendil-works/pi-coding-agent"
import { assertAgentSessionEventContract } from "./event-contract.ts"
import {
  asTranscriptRecord as asRecord,
  assistantContentParts as assistantParts,
  assistantErrorMessage as errorMessage,
  boundedTranscriptValue as boundedUnknown,
  formatToolTranscriptSummary as formatToolSummary,
  limitTranscriptContent as limitContent,
  transcriptMessageRole as messageRole,
  transcriptTextParts as textParts,
} from "./transcript-values.ts"
import { isReadToolName } from "./tool-names.ts"

export type TranscriptRowKind =
  | "user"
  | "assistant"
  | "tool"
  | "notice"
  | "error"

interface TranscriptRowBase {
  readonly id: string
  readonly kind: TranscriptRowKind
  readonly title: string
  readonly content: string
  readonly pending: boolean
  readonly isError: boolean
}

export interface AssistantTranscriptRow extends TranscriptRowBase {
  readonly kind: "assistant"
  readonly thinking: string
  readonly thinkingRedacted: boolean
}

export interface ToolTranscriptRow extends TranscriptRowBase {
  readonly kind: "tool"
  readonly toolCallId: string
  readonly toolName: string
  readonly args: unknown
  readonly partialResult: unknown
  readonly result: unknown
  readonly readGroupId?: string
}

export interface TextTranscriptRow extends TranscriptRowBase {
  readonly kind: "user" | "notice" | "error"
}

export type TranscriptRow =
  | AssistantTranscriptRow
  | ToolTranscriptRow
  | TextTranscriptRow

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


const assistantRow = (
  id: string,
  message: unknown,
  pending: boolean,
): AssistantTranscriptRow => {
  const parts = assistantParts(message)
  const fault = errorMessage(message)
  return {
    id,
    kind: "assistant",
    title: "pi",
    content: [parts.text, fault]
      .filter((part): part is string => part !== undefined && part.length > 0)
      .join("\n"),
    thinking: parts.thinking,
    thinkingRedacted: parts.thinkingRedacted,
    pending,
    isError: fault !== undefined,
  }
}

const hasAssistantDisplay = (row: AssistantTranscriptRow): boolean =>
  row.content.length > 0 ||
  row.thinking.length > 0 ||
  row.thinkingRedacted

const toolRow = (
  toolCallId: string,
  toolName: string,
  args: unknown,
  pending: boolean,
  readGroupId?: string,
): ToolTranscriptRow => {
  const boundedArgs = boundedUnknown(args)
  return {
    id: `tool-${toolCallId}`,
    kind: "tool",
    title: toolName,
    content: formatToolSummary(boundedArgs),
    pending,
    isError: false,
    toolCallId,
    toolName,
    args: boundedArgs,
    partialResult: undefined,
    result: undefined,
    ...(readGroupId === undefined ? {} : { readGroupId }),
  }
}

interface GroupedAssistantToolCall {
  readonly id: string
  readonly name: string
  readonly arguments: unknown
  readonly readGroupId?: string
}

const groupedAssistantToolCalls = (
  message: unknown,
): ReadonlyArray<GroupedAssistantToolCall> => {
  const content = asRecord(message)?.content
  if (!Array.isArray(content)) return []

  const calls: Array<GroupedAssistantToolCall> = []
  let readGroupId: string | undefined
  for (const part of content) {
    const value = asRecord(part)
    if (
      value?.type !== "toolCall" ||
      typeof value.id !== "string" ||
      typeof value.name !== "string"
    ) {
      readGroupId = undefined
      continue
    }
    if (isReadToolName(value.name)) {
      readGroupId ??= value.id
    } else {
      readGroupId = undefined
    }
    calls.push({
      id: value.id,
      name: value.name,
      arguments: value.arguments,
      ...(isReadToolName(value.name) && readGroupId !== undefined
        ? { readGroupId }
        : {}),
    })
  }
  return calls
}

const appendAssistantToolCalls = (
  model: TranscriptModel,
  message: unknown,
): TranscriptModel =>
  groupedAssistantToolCalls(message).reduce((current, call) => {
    const next = toolRow(
      call.id,
      call.name,
      call.arguments,
      true,
      call.readGroupId,
    )
    return current.rows.some((row) => row.id === next.id)
      ? updateRow(current, next.id, () => next)
      : appendRow(current, next)
  }, model)

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

const appendSavedAssistant = (
  model: TranscriptModel,
  message: unknown,
): TranscriptModel => {
  const row = assistantRow(`row-${model.nextRowId}`, message, false)
  const withAssistant = hasAssistantDisplay(row)
    ? appendRow(model, row)
    : model

  return appendAssistantToolCalls(withAssistant, message)
}

const finishToolRow = (
  model: TranscriptModel,
  toolCallId: string,
  toolName: string,
  result: unknown,
  isError: boolean,
): TranscriptModel => {
  const id = `tool-${toolCallId}`
  const boundedResult = boundedUnknown(result)
  if (!model.rows.some((row) => row.id === id)) {
    return appendRow(
      model,
      {
        ...toolRow(toolCallId, toolName, {}, false),
        content: formatToolSummary(boundedResult),
        result: boundedResult,
        isError,
      },
    )
  }
  return updateRow(model, id, (row) =>
    row.kind === "tool"
      ? {
          ...row,
          content: formatToolSummary(boundedResult),
          result: boundedResult,
          pending: false,
          isError,
        }
      : row,
  )
}

export const transcriptFromMessages = (
  messages: ReadonlyArray<unknown>,
): TranscriptModel => {
  const replayed = messages.reduce<TranscriptModel>((model, message) => {
    const record = asRecord(message)
    const role = messageRole(message)

    if (role === "user") {
      const text = textParts(record?.content).join("\n")
      return text.length > 0 ? appendUserPrompt(model, text) : model
    }

    if (role === "assistant") return appendSavedAssistant(model, message)

    if (
      role === "toolResult" &&
      typeof record?.toolCallId === "string"
    ) {
      return finishToolRow(
        model,
        record.toolCallId,
        typeof record.toolName === "string" ? record.toolName : "tool",
        message,
        record.isError === true,
      )
    }

    if (
      (role === "compactionSummary" || role === "branchSummary") &&
      typeof record?.summary === "string"
    ) {
      return appendRow(model, {
        id: `row-${model.nextRowId}`,
        kind: "notice",
        title:
          role === "compactionSummary"
            ? "compaction summary"
            : "branch summary",
        content: limitContent(record.summary),
        pending: false,
        isError: false,
      })
    }

    if (
      role === "bashExecution" &&
      typeof record?.command === "string"
    ) {
      const output = typeof record.output === "string" ? record.output : ""
      const failed =
        typeof record.exitCode === "number" && record.exitCode !== 0
      const callId = `bash-${model.nextRowId}`
      return appendRow(model, {
        ...toolRow(callId, "bash", { command: record.command }, false),
        content: formatToolSummary(output),
        result: boundedUnknown({
          content: [{ type: "text", text: output }],
        }),
        isError: failed,
      })
    }

    if (role === "custom" && record?.display === true) {
      const content = textParts(record.content).join("\n")
      if (content.length === 0) return model
      return appendRow(model, {
        id: `row-${model.nextRowId}`,
        kind: "notice",
        title:
          typeof record.customType === "string"
            ? record.customType
            : "extension",
        content: limitContent(content),
        pending: false,
        isError: false,
      })
    }

    return model
  }, emptyTranscript)
  return {
    ...replayed,
    rows: replayed.rows.map((row) =>
      row.kind === "tool" && row.pending
        ? {
            ...row,
            content: "Tool did not finish",
            pending: false,
            isError: true,
            result: {
              content: [{ type: "text", text: "Tool did not finish" }],
            },
          }
        : row,
    ),
  }
}

export const reduceTranscriptEvent = (
  model: TranscriptModel,
  event: AgentSessionEvent,
): TranscriptModel => {
  assertAgentSessionEventContract(event)

  switch (event.type) {
    case "message_start": {
      if (messageRole(event.message) !== "assistant") return model

      const id = `row-${model.nextRowId}`
      const row = assistantRow(id, event.message, true)
      if (!hasAssistantDisplay(row)) return model
      return {
        ...appendRow(model, row),
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
      return updateRow(model, model.activeAssistantId, (row) =>
        row.kind === "assistant"
          ? assistantRow(row.id, event.message, true)
          : row,
      )
    }

    case "message_end": {
      if (messageRole(event.message) !== "assistant") return model

      const activeId = model.activeAssistantId
      if (activeId === undefined) {
        const row = assistantRow(
          `row-${model.nextRowId}`,
          event.message,
          false,
        )
        const withAssistant = hasAssistantDisplay(row)
          ? appendRow(model, row)
          : model
        return appendAssistantToolCalls(withAssistant, event.message)
      }
      const ended = {
        ...updateRow(model, activeId, (row) =>
          row.kind === "assistant"
            ? assistantRow(row.id, event.message, false)
            : row,
        ),
        activeAssistantId: undefined,
      }
      return appendAssistantToolCalls(ended, event.message)
    }

    case "tool_execution_start": {
      const row = toolRow(
        event.toolCallId,
        event.toolName,
        event.args,
        true,
      )
      return model.rows.some((current) => current.id === row.id)
        ? updateRow(model, row.id, (current) =>
            current.kind === "tool" && current.readGroupId !== undefined
              ? { ...row, readGroupId: current.readGroupId }
              : row
          )
        : appendRow(model, row)
    }

    case "tool_execution_update": {
      const id = `tool-${event.toolCallId}`
      const partialResult = boundedUnknown(event.partialResult)
      const args = boundedUnknown(event.args)
      if (!model.rows.some((row) => row.id === id)) {
        return appendRow(
          model,
          {
            ...toolRow(event.toolCallId, event.toolName, event.args, true),
            content: formatToolSummary(partialResult),
            args,
            partialResult,
          },
        )
      }
      return updateRow(model, id, (row) =>
        row.kind === "tool"
          ? {
              ...row,
              content: formatToolSummary(partialResult),
              args,
              partialResult,
            }
          : row,
      )
    }

    case "tool_execution_end":
      return finishToolRow(
        model,
        event.toolCallId,
        event.toolName,
        event.result,
        event.isError,
      )

    case "compaction_start":
      return appendTranscriptNotice(
        model,
        `Compaction started (${event.reason})`,
      )

    case "compaction_end":
      return appendTranscriptNotice(
        model,
        event.errorMessage ??
          (event.aborted ? "Compaction stopped" : "Compaction finished"),
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

    case "agent_start":
    case "agent_end":
    case "agent_settled":
    case "turn_start":
    case "turn_end":
    case "queue_update":
    case "entry_appended":
    case "session_info_changed":
    case "thinking_level_changed":
      return model
  }
}
