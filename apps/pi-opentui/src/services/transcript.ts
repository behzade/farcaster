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
import type {
  ExtensionToolPresentation,
  PresentExtensionTool,
} from "./extension-tool-presentation.ts"

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
  readonly extensionPresentation?: ExtensionToolPresentation
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
  readonly nextRowId: number
}

export const emptyTranscript: TranscriptModel = {
  rows: [],
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

const removeRow = (
  model: TranscriptModel,
  id: string,
): TranscriptModel => ({
  ...model,
  rows: model.rows.filter((row) => row.id !== id),
})

const withExtensionPresentation = (
  row: ToolTranscriptRow,
  rendererArgs: unknown,
  rendererResult: unknown,
  pending: boolean,
  isError: boolean,
  presentExtensionTool: PresentExtensionTool | undefined,
): ToolTranscriptRow => {
  const extensionPresentation = presentExtensionTool?.({
    toolCallId: row.toolCallId,
    toolName: row.toolName,
    args: rendererArgs,
    result: rendererResult,
    pending,
    isError,
  })
  return extensionPresentation === undefined
    ? row
    : { ...row, extensionPresentation }
}

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

const activeAssistant = (
  model: TranscriptModel,
): AssistantTranscriptRow | undefined => {
  for (let index = model.rows.length - 1; index >= 0; index -= 1) {
    const row = model.rows[index]
    if (row?.kind === "assistant" && row.pending) return row
  }
  return undefined
}

const toolRow = (
  toolCallId: string,
  toolName: string,
  args: unknown,
  pending: boolean,
  readGroupId?: string,
  presentExtensionTool?: PresentExtensionTool,
): ToolTranscriptRow => {
  const boundedArgs = boundedUnknown(args)
  const row: ToolTranscriptRow = {
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
  return withExtensionPresentation(
    row,
    args,
    undefined,
    pending,
    false,
    presentExtensionTool,
  )
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
  presentExtensionTool?: PresentExtensionTool,
): TranscriptModel =>
  groupedAssistantToolCalls(message).reduce((current, call) => {
    const next = toolRow(
      call.id,
      call.name,
      call.arguments,
      true,
      call.readGroupId,
      presentExtensionTool,
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
  presentExtensionTool?: PresentExtensionTool,
): TranscriptModel => {
  const row = assistantRow(`row-${model.nextRowId}`, message, false)
  const withAssistant = hasAssistantDisplay(row)
    ? appendRow(model, row)
    : model

  return appendAssistantToolCalls(withAssistant, message, presentExtensionTool)
}

const finishToolRow = (
  model: TranscriptModel,
  toolCallId: string,
  toolName: string,
  result: unknown,
  isError: boolean,
  presentExtensionTool?: PresentExtensionTool,
): TranscriptModel => {
  const id = `tool-${toolCallId}`
  const boundedResult = boundedUnknown(result)
  if (!model.rows.some((row) => row.id === id)) {
    const base = toolRow(
      toolCallId,
      toolName,
      {},
      false,
      undefined,
      undefined,
    )
    return appendRow(
      model,
      withExtensionPresentation(
        {
          ...base,
          content: formatToolSummary(boundedResult),
          result: boundedResult,
          isError,
        },
        base.args,
        result,
        false,
        isError,
        presentExtensionTool,
      ),
    )
  }
  return updateRow(model, id, (row) =>
    row.kind === "tool"
      ? withExtensionPresentation(
          {
            ...row,
            content: formatToolSummary(boundedResult),
            result: boundedResult,
            pending: false,
            isError,
          },
          row.args,
          result,
          false,
          isError,
          presentExtensionTool,
        )
      : row,
  )
}

export const transcriptFromMessages = (
  messages: ReadonlyArray<unknown>,
  presentExtensionTool?: PresentExtensionTool,
): TranscriptModel => {
  const replayed = messages.reduce<TranscriptModel>((model, message) => {
    const record = asRecord(message)
    const role = messageRole(message)

    if (role === "user") {
      const text = textParts(record?.content).join("\n")
      return text.length > 0 ? appendUserPrompt(model, text) : model
    }

    if (role === "assistant") {
      return appendSavedAssistant(model, message, presentExtensionTool)
    }

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
        presentExtensionTool,
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
  presentExtensionTool?: PresentExtensionTool,
): TranscriptModel => {
  assertAgentSessionEventContract(event)

  switch (event.type) {
    case "turn_start": {
      if (activeAssistant(model) !== undefined) return model
      const id = `row-${model.nextRowId}`
      return appendRow(
        model,
        assistantRow(id, { role: "assistant", content: [] }, true),
      )
    }

    case "message_start": {
      const role = messageRole(event.message)
      if (role === "user") {
        const text = textParts(asRecord(event.message)?.content).join("\n")
        return text.length > 0 ? appendUserPrompt(model, text) : model
      }
      if (role !== "assistant") return model

      const active = activeAssistant(model)
      if (active !== undefined) {
        return updateRow(model, active.id, (row) =>
          row.kind === "assistant"
            ? assistantRow(row.id, event.message, true)
            : row,
        )
      }
      const id = `row-${model.nextRowId}`
      const row = assistantRow(id, event.message, true)
      return appendRow(model, row)
    }

    case "message_update": {
      if (messageRole(event.message) !== "assistant") return model

      const active = activeAssistant(model)
      if (active === undefined) {
        return reduceTranscriptEvent(model, {
          type: "message_start",
          message: event.message,
        }, presentExtensionTool)
      }
      return updateRow(model, active.id, (row) =>
        row.kind === "assistant"
          ? assistantRow(row.id, event.message, true)
          : row,
      )
    }

    case "message_end": {
      if (messageRole(event.message) !== "assistant") return model

      const active = activeAssistant(model)
      if (active === undefined) {
        const row = assistantRow(
          `row-${model.nextRowId}`,
          event.message,
          false,
        )
        const withAssistant = hasAssistantDisplay(row)
          ? appendRow(model, row)
          : model
        return appendAssistantToolCalls(
          withAssistant,
          event.message,
          presentExtensionTool,
        )
      }
      const endedRow = assistantRow(active.id, event.message, false)
      const ended = updateRow(model, active.id, (row) =>
        row.kind === "assistant" ? endedRow : row,
      )
      return appendAssistantToolCalls(
        hasAssistantDisplay(endedRow)
          ? ended
          : removeRow(ended, active.id),
        event.message,
        presentExtensionTool,
      )
    }

    case "tool_execution_start": {
      const row = toolRow(
        event.toolCallId,
        event.toolName,
        event.args,
        true,
        undefined,
        presentExtensionTool,
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
        const base = toolRow(
          event.toolCallId,
          event.toolName,
          event.args,
          true,
          undefined,
          undefined,
        )
        return appendRow(
          model,
          withExtensionPresentation(
            {
              ...base,
              content: formatToolSummary(partialResult),
              args,
              partialResult,
            },
            event.args,
            event.partialResult,
            true,
            false,
            presentExtensionTool,
          ),
        )
      }
      return updateRow(model, id, (row) =>
        row.kind === "tool"
          ? withExtensionPresentation(
              {
                ...row,
                content: formatToolSummary(partialResult),
                args,
                partialResult,
              },
              event.args,
              event.partialResult,
              true,
              false,
              presentExtensionTool,
            )
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
        presentExtensionTool,
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
    case "turn_end":
    case "queue_update":
    case "entry_appended":
    case "session_info_changed":
    case "thinking_level_changed":
      return model
  }
}
