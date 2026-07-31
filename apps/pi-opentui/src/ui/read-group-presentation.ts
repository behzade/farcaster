import type {
  ToolPresentation,
  ToolRunState,
} from "./tool-presentation.ts"

export interface ReadGroupPresentationEntry {
  readonly id: string
  readonly args: unknown
  readonly result?: unknown
  readonly pending: boolean
  readonly isError: boolean
}

export interface ReadGroupPresentationOptions {
  readonly homeDirectory?: string
}

type UnknownRecord = Record<string, unknown>

const asRecord = (value: unknown): UnknownRecord | undefined =>
  typeof value === "object" && value !== null
    ? (value as UnknownRecord)
    : undefined

const readPath = (args: unknown): string => {
  const record = asRecord(args)
  for (const name of ["path", "file_path", "filePath"] as const) {
    const value = record?.[name]
    if (typeof value === "string") return value
  }
  return "file"
}

const readRange = (args: unknown): string => {
  const record = asRecord(args)
  const offset = typeof record?.offset === "number" ? record.offset : undefined
  const limit = typeof record?.limit === "number" ? record.limit : undefined
  if (offset === undefined && limit === undefined) return ""

  const start = offset ?? 1
  const end = limit === undefined ? "" : `-${start + limit - 1}`
  return `:${start}${end}`
}

const resultContent = (result: unknown): ReadonlyArray<unknown> => {
  const content = asRecord(result)?.content
  return Array.isArray(content) ? content : []
}

const textResult = (result: unknown): string | undefined => {
  if (typeof result === "string") return result
  const block = resultContent(result).find(
    (item) => asRecord(item)?.type === "text",
  )
  const text = asRecord(block)?.text
  return typeof text === "string" ? text : undefined
}

const hasImage = (result: unknown): boolean =>
  resultContent(result).some(
    (item) => asRecord(item)?.type === "image",
  )

const resultLabel = (entry: ReadGroupPresentationEntry): string => {
  if (entry.pending || entry.result === undefined) return "…"
  if (entry.isError) return "✗"
  if (hasImage(entry.result)) return "image"

  const text = textResult(entry.result)
  return text === undefined ? "✓" : `${text.split("\n").length} lines`
}

const marker = (
  entry: ReadGroupPresentationEntry,
): "…" | "✓" | "✗" =>
  entry.isError ? "✗" : entry.pending ? "…" : "✓"

const displayPath = (path: string, homeDirectory: string | undefined): string =>
  homeDirectory !== undefined && path.startsWith(`${homeDirectory}/`)
    ? `~/${path.slice(homeDirectory.length + 1)}`
    : path

const pathAndRange = (
  entry: ReadGroupPresentationEntry,
  homeDirectory: string | undefined,
): string =>
  `${displayPath(readPath(entry.args), homeDirectory)}${readRange(entry.args)}`

const groupState = (
  entries: ReadonlyArray<ReadGroupPresentationEntry>,
): ToolRunState => {
  if (entries.some((entry) => entry.isError)) return "error"
  if (entries.some((entry) => entry.pending)) return "pending"
  return "complete"
}

export const readGroupPresentation = (
  entries: ReadonlyArray<ReadGroupPresentationEntry>,
  options: ReadGroupPresentationOptions = {},
): ToolPresentation => {
  if (entries.length === 0) {
    throw new Error("A read group must contain at least one entry")
  }

  const state = groupState(entries)
  if (entries.length === 1) {
    const entry = entries[0]!
    return {
      toolName: "read",
      canonicalTool: "read",
      title: "read",
      detail: `${pathAndRange(entry, options.homeDirectory)}  ${resultLabel(entry)}`,
      state,
      showStateLabel: false,
      body: { kind: "none" },
    }
  }

  const done = entries.filter(
    (entry) => !entry.pending && !entry.isError,
  ).length
  const failed = entries.filter((entry) => entry.isError).length
  const detail = `${done}/${entries.length}${
    failed > 0 ? `, ${failed} failed` : ""
  }`
  return {
    toolName: "read",
    canonicalTool: "read",
    title: `read ${entries.length} files`,
    detail,
    state,
    showStateLabel: false,
    body: {
      kind: "read-group",
      entries: entries.map((entry) => ({
        marker: marker(entry),
        path: pathAndRange(entry, options.homeDirectory),
        label: resultLabel(entry),
        state: entry.isError
          ? "error"
          : entry.pending
            ? "pending"
            : "complete",
      })),
    },
  }
}
