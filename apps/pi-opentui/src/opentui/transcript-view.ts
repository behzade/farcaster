import {
  BoxRenderable,
  TextAttributes,
  TextRenderable,
  type RenderContext,
} from "@opentui/core"
import type {
  TranscriptModel,
  TranscriptRow,
  TranscriptRowKind,
} from "../services/transcript.ts"
import type { OpenTuiComponent } from "./component.ts"
import { theme } from "./theme.ts"

const rowColor = (kind: TranscriptRowKind, isError: boolean): string => {
  if (isError || kind === "error") return theme.error
  switch (kind) {
    case "user":
      return theme.user
    case "assistant":
      return theme.assistant
    case "tool":
      return theme.tool
    case "notice":
      return theme.accent
  }
}
class TranscriptRowView {
  readonly root: BoxRenderable

  private readonly title: TextRenderable
  private readonly content: TextRenderable

  constructor(ctx: RenderContext, row: TranscriptRow) {
    this.root = new BoxRenderable(ctx, {
      flexDirection: "column",
      paddingLeft: 1,
      paddingRight: 1,
    })
    this.title = new TextRenderable(ctx, {
      content: "",
      attributes: TextAttributes.BOLD,
    })
    this.content = new TextRenderable(ctx, {
      content: "",
      wrapMode: "word",
    })
    this.root.add(this.title)
    this.root.add(this.content)
    this.update(undefined, row)
  }

  update(previous: TranscriptRow | undefined, current: TranscriptRow): void {
    if (
      previous === undefined ||
      previous.title !== current.title ||
      previous.pending !== current.pending
    ) {
      this.title.content = `${current.title}${current.pending ? " …" : ""}`
    }
    if (
      previous === undefined ||
      previous.kind !== current.kind ||
      previous.isError !== current.isError
    ) {
      this.title.fg = rowColor(current.kind, current.isError)
      this.content.fg = current.kind === "tool" ? "#a89984" : theme.text
    }
    if (previous === undefined || previous.content !== current.content) {
      this.content.content = current.content
    }
  }
}

export class TranscriptView implements OpenTuiComponent<TranscriptModel> {
  readonly root: BoxRenderable

  private readonly empty: TextRenderable
  private readonly rows = new Map<
    string,
    { readonly view: TranscriptRowView; row: TranscriptRow }
  >()
  private rowOrder: ReadonlyArray<string> = []

  constructor(private readonly ctx: RenderContext) {
    this.root = new BoxRenderable(ctx, {
      flexDirection: "column",
      gap: 1,
    })
    this.empty = new TextRenderable(ctx, {
      content: "",
      fg: theme.muted,
    })
    this.root.add(this.empty)
  }

  update(previous: TranscriptModel | undefined, current: TranscriptModel): void {
    if (previous?.rows === current.rows) return
    this.empty.content =
      current.rows.length === 0 ? "Type a prompt to start." : ""

    const nextOrder = current.rows.map((row) => row.id)
    const keepsOldOrder = this.rowOrder.every(
      (id, index) => nextOrder[index] === id,
    )
    if (!keepsOldOrder) this.clearRows()

    for (const row of current.rows) {
      const entry = this.rows.get(row.id)
      if (entry === undefined) {
        const view = new TranscriptRowView(this.ctx, row)
        this.rows.set(row.id, { view, row })
        this.root.add(view.root)
      } else {
        entry.view.update(entry.row, row)
        entry.row = row
      }
    }
    this.rowOrder = nextOrder
  }

  destroy(): void {
    this.rows.clear()
    this.rowOrder = []
    this.root.destroyRecursively()
  }

  private clearRows(): void {
    for (const entry of this.rows.values()) {
      entry.view.root.destroyRecursively()
    }
    this.rows.clear()
    this.rowOrder = []
  }
}
