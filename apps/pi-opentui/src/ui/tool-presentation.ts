import {
  canonicalToolName,
  type CanonicalToolName,
} from "../services/tool-names.ts"

export type { CanonicalToolName } from "../services/tool-names.ts"

export type ToolRunState = "pending" | "complete" | "error"

export type ToolPresentationBody =
  | {
      readonly kind: "none"
    }
  | {
      readonly kind: "text"
      readonly content: string
    }
  | {
      readonly kind: "code"
      readonly content: string
      readonly path?: string
      readonly language?: string
      readonly streaming: boolean
    }
  | {
      readonly kind: "markdown"
      readonly content: string
      readonly streaming: boolean
    }
  | {
      readonly kind: "diff"
      readonly patch: string
      readonly path?: string
      readonly showLineNumbers: boolean
    }
  | {
      readonly kind: "read-group"
      readonly entries: ReadonlyArray<{
        readonly marker: "…" | "✓" | "✗"
        readonly path: string
        readonly label: string
        readonly state: ToolRunState
      }>
    }

export interface ToolPresentation {
  readonly toolName: string
  readonly canonicalTool: CanonicalToolName
  readonly title: string
  readonly detail?: string
  readonly state: ToolRunState
  readonly showStateLabel?: boolean
  readonly body: ToolPresentationBody
}

export interface ToolPresentationInput {
  readonly toolName: string
  readonly args?: unknown
  readonly result?: unknown
  readonly pending?: boolean
  readonly isError?: boolean
  readonly extension?: {
    readonly label: string
    readonly call?: string
    readonly result?: string
  }
}

type UnknownRecord = Record<string, unknown>

const asRecord = (value: unknown): UnknownRecord | undefined =>
  typeof value === "object" && value !== null
    ? (value as UnknownRecord)
    : undefined

const stringField = (
  record: UnknownRecord | undefined,
  ...names: ReadonlyArray<string>
): string | undefined => {
  for (const name of names) {
    const value = record?.[name]
    if (typeof value === "string") return value
  }
  return undefined
}

const safeJson = (value: unknown): string => {
  if (value === undefined) return ""
  if (typeof value === "string") return value
  try {
    return JSON.stringify(value, null, 2) ?? String(value)
  } catch {
    return String(value)
  }
}

const textContent = (value: unknown): string | undefined => {
  if (typeof value === "string") return value
  const record = asRecord(value)
  const content = record?.content
  if (typeof content === "string") return content
  if (Array.isArray(content)) {
    const parts = content.flatMap((part) => {
      if (typeof part === "string") return [part]
      const item = asRecord(part)
      return typeof item?.text === "string" ? [item.text] : []
    })
    if (parts.length > 0) return parts.join("\n")
  }
  return stringField(record, "output", "message")
}

const previewLines = (value: string): ReadonlyArray<string> => {
  const normalized = value.replace(/\r\n?/g, "\n")
  if (normalized.length === 0) return []
  const lines = normalized.split("\n")
  while (lines.at(-1) === "") lines.pop()
  return lines
}

const headPreview = (value: string, limit: number): string => {
  const lines = previewLines(value)
  if (lines.length <= limit) return lines.join("\n")
  const omitted = lines.length - limit
  return [
    ...lines.slice(0, limit),
    `… (${omitted} more lines, ${lines.length} total)`,
  ].join("\n")
}

const tailPreview = (value: string, limit: number): string => {
  const lines = previewLines(value)
  if (lines.length <= limit) return lines.join("\n")
  const omitted = lines.length - limit
  return [
    `… (${omitted} earlier lines)`,
    ...lines.slice(-limit),
  ].join("\n")
}

const characterTail = (value: string, limit: number): string => {
  const characters = Array.from(value)
  if (characters.length <= limit) return value
  const omitted = characters.length - limit
  return `… (${omitted} earlier chars)\n${characters.slice(-limit).join("")}`
}

const characterHead = (value: string, limit: number): string => {
  const characters = Array.from(value)
  if (characters.length <= limit) return value
  const omitted = characters.length - limit
  return `${characters.slice(0, limit).join("")}\n… (${omitted} more chars)`
}

const toolState = (input: ToolPresentationInput): ToolRunState => {
  if (input.isError === true || asRecord(input.result)?.isError === true) {
    return "error"
  }
  return input.pending === true ? "pending" : "complete"
}

const pathFromArgs = (args: unknown): string | undefined =>
  stringField(asRecord(args), "path", "file_path", "filePath")

const lineRange = (args: unknown): string | undefined => {
  const record = asRecord(args)
  const offset = typeof record?.offset === "number" ? record.offset : undefined
  const limit = typeof record?.limit === "number" ? record.limit : undefined
  if (offset === undefined && limit === undefined) return undefined
  const start = offset ?? 1
  return limit === undefined
    ? `lines ${start}+`
    : `lines ${start}-${start + Math.max(0, limit - 1)}`
}

const cleanPatchPath = (path: string | undefined): string =>
  (path ?? "file").replace(/[\r\n]/g, " ")

const diffLines = (value: string): ReadonlyArray<string> => {
  const normalized = value.replace(/\r\n?/g, "\n")
  if (normalized.length === 0) return []
  const withoutLastNewline = normalized.endsWith("\n")
    ? normalized.slice(0, -1)
    : normalized
  return withoutLastNewline.split("\n")
}

const diffRange = (count: number): string =>
  count === 1 ? "1" : `1,${count}`

interface EditReplacement {
  readonly oldText: string
  readonly newText: string
}

const editReplacements = (args: unknown): ReadonlyArray<EditReplacement> => {
  const record = asRecord(args)
  if (Array.isArray(record?.edits)) {
    const edits = record.edits.flatMap((value) => {
      const edit = asRecord(value)
      return typeof edit?.oldText === "string" &&
          typeof edit.newText === "string"
        ? [{ oldText: edit.oldText, newText: edit.newText }]
        : []
    })
    if (edits.length > 0) return edits
  }
  return typeof record?.oldText === "string" &&
      typeof record.newText === "string"
    ? [{ oldText: record.oldText, newText: record.newText }]
    : []
}

/**
 * Builds a display-only patch from edit call arguments. The call does not
 * contain source line positions, so the view hides line numbers for this form.
 * Completed edit results should prefer the SDK's exact `details.patch`.
 */
export const patchFromEditArguments = (
  path: string | undefined,
  args: unknown,
): string | undefined => {
  const edits = editReplacements(args)
  if (edits.length === 0) return undefined
  const safePath = cleanPatchPath(path)
  const hunks = edits.map((edit) => {
    const oldLines = diffLines(edit.oldText)
    const newLines = diffLines(edit.newText)
    return [
      `@@ -${diffRange(oldLines.length)} +${diffRange(newLines.length)} @@`,
      ...oldLines.map((line) => `-${line}`),
      ...newLines.map((line) => `+${line}`),
    ].join("\n")
  })
  return [
    `--- a/${safePath}`,
    `+++ b/${safePath}`,
    ...hunks,
    "",
  ].join("\n")
}

const patchFromResult = (result: unknown): string | undefined => {
  const record = asRecord(result)
  const details = asRecord(record?.details)
  return stringField(details, "patch") ?? stringField(record, "patch")
}

const resultOrFallback = (input: ToolPresentationInput): string =>
  textContent(input.result) ?? safeJson(input.result ?? input.args)

const presentRead = (
  input: ToolPresentationInput,
  state: ToolRunState,
): ToolPresentation => {
  const path = pathFromArgs(input.args)
  const range = lineRange(input.args)
  const detail = [path, range].filter(Boolean).join(" · ") || undefined
  const content = textContent(input.result) ?? safeJson(input.result)
  const body: ToolPresentationBody = state === "error"
    ? { kind: "text", content: headPreview(content, 10) }
    : { kind: "none" }
  return {
    toolName: input.toolName,
    canonicalTool: "read",
    title: "read",
    ...(detail === undefined ? {} : { detail }),
    state,
    body,
  }
}

const presentWrite = (
  input: ToolPresentationInput,
  state: ToolRunState,
): ToolPresentation => {
  const args = asRecord(input.args)
  const path = pathFromArgs(input.args)
  const content = stringField(args, "content") ?? resultOrFallback(input)
  const preview = headPreview(content.replace(/\t/g, "   "), 10)
  return {
    toolName: input.toolName,
    canonicalTool: "write",
    title: "write",
    ...(path === undefined ? {} : { detail: path }),
    state,
    body: state === "error"
      ? { kind: "text", content: headPreview(resultOrFallback(input), 10) }
      : preview.length === 0
        ? { kind: "none" }
        : {
          kind: "code",
          content: preview,
          ...(path === undefined ? {} : { path }),
          streaming: state === "pending",
        },
  }
}

const presentEdit = (
  input: ToolPresentationInput,
  state: ToolRunState,
): ToolPresentation => {
  const path = pathFromArgs(input.args)
  const resultPatch = patchFromResult(input.result)
  const patch = resultPatch ?? patchFromEditArguments(path, input.args)
  const patchWasCut = patch?.includes("\n…") === true
  return {
    toolName: input.toolName,
    canonicalTool: "edit",
    title: "edit",
    ...(path === undefined ? {} : { detail: path }),
    state,
    body: state === "error" || patch === undefined || patchWasCut
      ? {
          kind: "text",
          content: patchWasCut && patch !== undefined
            ? patch
            : resultOrFallback(input),
        }
      : {
          kind: "diff",
          patch,
          ...(path === undefined ? {} : { path }),
          showLineNumbers: resultPatch !== undefined,
        },
  }
}

const presentBash = (
  input: ToolPresentationInput,
  state: ToolRunState,
): ToolPresentation => {
  const args = asRecord(input.args)
  const command = stringField(args, "command") ?? ""
  const commandPreview = characterHead(headPreview(command.trim(), 3), 320)
  const output = (textContent(input.result) ?? "").trim()
  const outputPreview = tailPreview(characterTail(output, 400), 5)
  const timeout = typeof args?.timeout === "number"
    ? `timeout ${args.timeout}s`
    : undefined
  return {
    toolName: input.toolName,
    canonicalTool: "bash",
    title: commandPreview.length > 0 ? `$ ${commandPreview}` : "bash",
    ...(timeout === undefined ? {} : { detail: timeout }),
    state,
    body: outputPreview.length === 0
      ? { kind: "none" }
      : {
          kind: "text",
          content: outputPreview,
        },
  }
}

const presentGeneric = (
  input: ToolPresentationInput,
  state: ToolRunState,
): ToolPresentation => {
  const renderedCall = input.extension?.call?.trim() ?? ""
  const renderedResult = input.extension?.result?.trim() ?? ""
  const callLines = previewLines(renderedCall)
  const customBody = renderedResult.length > 0
    ? renderedResult
    : callLines.slice(1).join("\n")
  if (input.extension !== undefined) {
    return {
      toolName: input.toolName,
      canonicalTool: "generic",
      title:
        callLines[0] ??
        (input.extension.label.trim() || input.toolName || "tool"),
      state,
      body: customBody.length > 0
        ? { kind: "text", content: customBody }
        : { kind: "none" },
    }
  }
  const args = safeJson(input.args)
  const output = textContent(input.result) ?? safeJson(input.result)
  const content = output.length === 0
    ? args
    : args.length === 0
      ? output
      : `input\n${args}\n\noutput\n${output}`
  return {
    toolName: input.toolName,
    canonicalTool: "generic",
    title: input.toolName.trim().length > 0 ? input.toolName : "tool",
    state,
    body: { kind: "text", content },
  }
}

export const toolPresentation = (
  input: ToolPresentationInput,
): ToolPresentation => {
  const state = toolState(input)
  switch (canonicalToolName(input.toolName)) {
    case "read":
      return presentRead(input, state)
    case "write":
      return presentWrite(input, state)
    case "edit":
      return presentEdit(input, state)
    case "bash":
      return presentBash(input, state)
    case "generic":
      return presentGeneric(input, state)
  }
}
