import { expect, test } from "bun:test"
import type {
  ToolTranscriptRow,
  TranscriptRow,
} from "../src/services/transcript.ts"
import { transcriptDisplayItems } from "../src/ui/transcript-groups.ts"

const read = (
  id: string,
  readGroupId: string | undefined,
): ToolTranscriptRow => ({
  id: `tool-${id}`,
  kind: "tool",
  title: "read",
  content: "",
  pending: true,
  isError: false,
  toolCallId: id,
  toolName: "read",
  args: { path: `${id}.ts` },
  partialResult: undefined,
  result: undefined,
  ...(readGroupId === undefined ? {} : { readGroupId }),
})

test("groups only reads claimed by the same source run", () => {
  const bash: ToolTranscriptRow = {
    ...read("bash", undefined),
    title: "bash",
    toolName: "bash",
    args: { command: "pwd" },
  }
  const rows: ReadonlyArray<TranscriptRow> = [
    read("a", "a"),
    read("b", "a"),
    read("c", "c"),
    bash,
    read("d", "d"),
    read("e", undefined),
  ]

  const items = transcriptDisplayItems(rows)
  expect(items.map((item) =>
    item.kind === "read-group"
      ? item.rows.map((row) => row.toolCallId)
      : item.row.id
  )).toEqual([
    ["a", "b"],
    ["c"],
    "tool-bash",
    ["d"],
    ["e"],
  ])
})
