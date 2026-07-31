import type {
  ToolTranscriptRow,
  TranscriptRow,
} from "../services/transcript.ts"
import { isReadToolName } from "../services/tool-names.ts"

export type TranscriptDisplayItem =
  | {
      readonly id: string
      readonly kind: "row"
      readonly row: TranscriptRow
    }
  | {
      readonly id: string
      readonly kind: "read-group"
      readonly rows: ReadonlyArray<ToolTranscriptRow>
    }

const isReadRow = (row: TranscriptRow): row is ToolTranscriptRow =>
  row.kind === "tool" && isReadToolName(row.toolName)

/** Uses the source assistant message's read-run ID; unclaimed reads stay single. */
export const transcriptDisplayItems = (
  rows: ReadonlyArray<TranscriptRow>,
): ReadonlyArray<TranscriptDisplayItem> => {
  const items: Array<TranscriptDisplayItem> = []
  for (let index = 0; index < rows.length;) {
    const row = rows[index]!
    if (!isReadRow(row)) {
      items.push({ id: row.id, kind: "row", row })
      index += 1
      continue
    }

    const reads: Array<ToolTranscriptRow> = [row]
    index += 1
    while (
      row.readGroupId !== undefined &&
      index < rows.length &&
      isReadRow(rows[index]!) &&
      (rows[index] as ToolTranscriptRow).readGroupId === row.readGroupId
    ) {
      reads.push(rows[index] as ToolTranscriptRow)
      index += 1
    }
    items.push({
      id: `read-group:${row.id}`,
      kind: "read-group",
      rows: reads,
    })
  }
  return items
}
