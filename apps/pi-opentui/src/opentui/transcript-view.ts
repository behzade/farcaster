import {
  BoxRenderable,
  TextAttributes,
  TextRenderable,
  type RenderContext,
  type SyntaxStyle,
  type TreeSitterClient,
} from "@opentui/core"
import { homedir } from "node:os"
import type {
  AssistantTranscriptRow,
  TextTranscriptRow,
  ToolTranscriptRow,
  TranscriptModel,
  TranscriptRow,
  TranscriptRowKind,
} from "../services/transcript.ts"
import { readGroupPresentation } from "../ui/read-group-presentation.ts"
import { toolPresentation } from "../ui/tool-presentation.ts"
import {
  transcriptDisplayItems,
  type TranscriptDisplayItem,
} from "../ui/transcript-groups.ts"
import type { OpenTuiComponent } from "./component.ts"
import {
  StableMarkdownView,
  type StableMarkdownModel,
} from "./stable-markdown-view.ts"
import { theme } from "./theme.ts"
import {
  createToolSyntaxStyle,
  ToolPresentationView,
} from "./tool-presentation-view.ts"

export interface TranscriptViewOptions {
  readonly treeSitterClient?: TreeSitterClient
  readonly hideThinkingBlock?: boolean
}

const workingFramesPerStep = 18

export const workingDots = (frame: number): string =>
  ".".repeat((Math.floor(frame / workingFramesPerStep) % 3) + 1)

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

interface TranscriptRowView {
  readonly root: BoxRenderable
  update(previous: TranscriptRow | undefined, current: TranscriptRow): void
  destroy(): void
}

class TextRowView implements TranscriptRowView {
  readonly root: BoxRenderable

  private readonly title: TextRenderable | undefined
  private readonly content: TextRenderable

  constructor(ctx: RenderContext, row: TextTranscriptRow) {
    const isUser = row.kind === "user"
    this.root = new BoxRenderable(ctx, {
      width: "100%",
      flexDirection: "column",
      paddingLeft: 1,
      paddingRight: 1,
      ...(isUser
        ? {
            paddingTop: 1,
            paddingBottom: 1,
            backgroundColor: theme.userMessageBg,
          }
        : {
            border: ["left" as const],
            borderColor: row.isError ? theme.error : theme.border,
          }),
    })
    this.title = isUser
      ? undefined
      : new TextRenderable(ctx, {
          content: "",
          attributes: TextAttributes.BOLD,
        })
    this.content = new TextRenderable(ctx, {
      content: "",
      wrapMode: "word",
    })
    if (this.title !== undefined) this.root.add(this.title)
    this.root.add(this.content)
    this.update(undefined, row)
  }

  update(previous: TranscriptRow | undefined, current: TranscriptRow): void {
    if (current.kind === "assistant" || current.kind === "tool") return
    if (
      previous === undefined ||
      previous.title !== current.title ||
      previous.pending !== current.pending
    ) {
      if (this.title !== undefined) {
        this.title.content = `${current.title}${current.pending ? " …" : ""}`
      }
    }
    if (
      previous === undefined ||
      previous.kind !== current.kind ||
      previous.isError !== current.isError
    ) {
      if (this.title !== undefined) {
        this.title.fg = rowColor(current.kind, current.isError)
      }
      this.content.fg = theme.text
    }
    if (previous === undefined || previous.content !== current.content) {
      this.content.content = current.content
    }
  }

  destroy(): void {
    this.root.destroyRecursively()
  }
}

class AssistantRowView implements TranscriptRowView {
  readonly root: BoxRenderable

  private readonly answer: StableMarkdownView
  private readonly working: TextRenderable
  private answerModel: StableMarkdownModel
  private thinking: StableMarkdownView | undefined
  private thinkingModel: StableMarkdownModel | undefined
  private current: AssistantTranscriptRow
  private workingActive = false
  private workingFrame = 0
  private workingText = ""

  private readonly handleWorkingFrame = (): void => {
    if (!this.workingActive) return
    this.workingFrame += 1
    const content = workingDots(this.workingFrame)
    if (this.workingText !== content) {
      this.workingText = content
      this.working.content = content
    }
  }

  constructor(
    private readonly ctx: RenderContext,
    private readonly syntaxStyle: SyntaxStyle,
    private readonly treeSitterClient: TreeSitterClient | undefined,
    row: AssistantTranscriptRow,
    private hideThinkingBlock: boolean,
  ) {
    this.current = row
    this.root = new BoxRenderable(ctx, {
      width: "100%",
      flexDirection: "column",
      paddingLeft: 1,
      paddingRight: 1,
    })
    this.answerModel = {
      content: "",
      color: theme.text,
      streaming: row.pending,
    }
    this.answer = new StableMarkdownView(ctx, this.answerModel, {
      syntaxStyle,
      ...(treeSitterClient === undefined ? {} : { treeSitterClient }),
    })
    this.root.add(this.answer.root)
    this.working = new TextRenderable(ctx, {
      content: "",
      fg: theme.muted,
      visible: false,
    })
    this.root.add(this.working)
    this.update(undefined, row)
  }

  update(previous: TranscriptRow | undefined, current: TranscriptRow): void {
    if (current.kind !== "assistant") return
    this.current = current
    const old = previous?.kind === "assistant" ? previous : undefined
    if (
      old === undefined ||
      old.pending !== current.pending ||
      old.isError !== current.isError ||
      old.content !== current.content
    ) {
      const nextAnswer: StableMarkdownModel = {
        content: current.content,
        color: current.isError ? theme.error : theme.text,
        streaming: current.pending,
      }
      this.answer.update(this.answerModel, nextAnswer)
      this.answerModel = nextAnswer
    }
    if (
      old === undefined ||
      old.thinking !== current.thinking ||
      old.thinkingRedacted !== current.thinkingRedacted ||
      old.content !== current.content ||
      old.pending !== current.pending
    ) {
      this.updateThinking(current)
    }
    this.updateWorking(current)
  }

  destroy(): void {
    this.setWorking(false)
    this.root.destroyRecursively()
    this.thinking = undefined
    this.thinkingModel = undefined
  }

  setHideThinkingBlock(hide: boolean): void {
    if (this.hideThinkingBlock === hide) return
    this.hideThinkingBlock = hide
    this.updateThinking(this.current)
  }

  private updateThinking(row: AssistantTranscriptRow): void {
    const content = this.hideThinkingBlock
      ? row.thinking.length > 0 || row.thinkingRedacted
        ? "Thinking…"
        : ""
      : [
          row.thinking,
          row.thinkingRedacted ? "[some thinking redacted by provider]" : "",
        ].filter(Boolean).join("\n")
    if (content.length === 0) {
      this.thinking?.destroy()
      this.thinking = undefined
      this.thinkingModel = undefined
      return
    }

    if (this.thinking === undefined) {
      this.thinkingModel = {
        content,
        color: theme.muted,
        streaming: row.pending,
      }
      this.thinking = new StableMarkdownView(
        this.ctx,
        this.thinkingModel,
        {
          syntaxStyle: this.syntaxStyle,
          ...(this.treeSitterClient === undefined
            ? {}
            : { treeSitterClient: this.treeSitterClient }),
        },
      )
      this.thinking.root.marginBottom = row.content.length > 0 ? 1 : 0
      this.root.insertBefore(this.thinking.root, this.answer.root)
      return
    }
    const nextThinking: StableMarkdownModel = {
      content,
      color: theme.muted,
      streaming: row.pending,
    }
    this.thinking.update(this.thinkingModel, nextThinking)
    this.thinkingModel = nextThinking
    this.thinking.root.marginBottom = row.content.length > 0 ? 1 : 0
  }

  private updateWorking(row: AssistantTranscriptRow): void {
    this.setWorking(
      row.pending &&
        row.content.length === 0 &&
        row.thinking.length === 0 &&
        !row.thinkingRedacted,
    )
  }

  private setWorking(active: boolean): void {
    if (this.workingActive === active) return
    this.workingActive = active
    if (active) {
      this.workingFrame = 0
      this.working.content = workingDots(0)
      this.working.visible = true
      this.ctx.on("frame", this.handleWorkingFrame)
      this.ctx.requestLive()
      return
    }
    this.ctx.off("frame", this.handleWorkingFrame)
    this.ctx.dropLive()
    this.working.visible = false
    this.working.content = ""
  }
}

class ToolRowView implements TranscriptRowView {
  readonly root: BoxRenderable

  private readonly view: ToolPresentationView

  constructor(
    ctx: RenderContext,
    syntaxStyle: SyntaxStyle,
    treeSitterClient: TreeSitterClient | undefined,
    row: ToolTranscriptRow,
  ) {
    this.view = new ToolPresentationView(ctx, this.model(row), {
      syntaxStyle,
      ...(treeSitterClient === undefined ? {} : { treeSitterClient }),
    })
    this.root = this.view.root
  }

  update(previous: TranscriptRow | undefined, current: TranscriptRow): void {
    if (current.kind !== "tool") return
    const old = previous?.kind === "tool" ? this.model(previous) : undefined
    this.view.update(old, this.model(current))
  }

  destroy(): void {
    this.view.destroy()
  }

  private model(row: ToolTranscriptRow) {
    return toolPresentation({
      toolName: row.toolName,
      args: row.args,
      result: row.pending ? row.partialResult : row.result,
      pending: row.pending,
      isError: row.isError,
      ...(row.extensionPresentation === undefined
        ? {}
        : { extension: row.extensionPresentation }),
    })
  }
}

class ReadGroupRowView {
  readonly root: BoxRenderable

  private readonly view: ToolPresentationView

  constructor(
    ctx: RenderContext,
    syntaxStyle: SyntaxStyle,
    treeSitterClient: TreeSitterClient | undefined,
    rows: ReadonlyArray<ToolTranscriptRow>,
  ) {
    this.view = new ToolPresentationView(ctx, this.model(rows), {
      syntaxStyle,
      ...(treeSitterClient === undefined ? {} : { treeSitterClient }),
    })
    this.root = this.view.root
  }

  update(
    previous: ReadonlyArray<ToolTranscriptRow> | undefined,
    current: ReadonlyArray<ToolTranscriptRow>,
  ): void {
    this.view.update(
      previous === undefined ? undefined : this.model(previous),
      this.model(current),
    )
  }

  destroy(): void {
    this.view.destroy()
  }

  private model(rows: ReadonlyArray<ToolTranscriptRow>) {
    return readGroupPresentation(
      rows.map((row) => ({
        id: row.toolCallId,
        args: row.args,
        result: row.pending ? row.partialResult : row.result,
        pending: row.pending,
        isError: row.isError,
      })),
      { homeDirectory: homedir() },
    )
  }
}

type TranscriptViewEntry =
  | {
      readonly kind: "row"
      readonly view: TranscriptRowView
      row: TranscriptRow
    }
  | {
      readonly kind: "read-group"
      readonly view: ReadGroupRowView
      rows: ReadonlyArray<ToolTranscriptRow>
    }

export class TranscriptView implements OpenTuiComponent<TranscriptModel> {
  readonly root: BoxRenderable

  private readonly empty: TextRenderable
  private readonly syntaxStyle = createToolSyntaxStyle()
  private readonly rows = new Map<
    string,
    TranscriptViewEntry
  >()
  private rowOrder: ReadonlyArray<string> = []
  private hideThinkingBlock: boolean

  constructor(
    private readonly ctx: RenderContext,
    private readonly options: TranscriptViewOptions = {},
  ) {
    this.hideThinkingBlock = options.hideThinkingBlock ?? false
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

    const items = transcriptDisplayItems(current.rows)
    const nextOrder = items.map((item) => item.id)
    const nextIds = new Set(nextOrder)
    const retainedOldOrder = this.rowOrder.filter((id) => nextIds.has(id))
    const retainedNextOrder = nextOrder.filter((id) => this.rows.has(id))
    const retainedOrderChanged =
      retainedOldOrder.length !== retainedNextOrder.length ||
      retainedOldOrder.some((id, index) => retainedNextOrder[index] !== id)
    if (retainedOrderChanged) {
      this.clearRows()
    } else {
      for (const [id, entry] of this.rows) {
        if (nextIds.has(id)) continue
        entry.view.destroy()
        this.rows.delete(id)
      }
    }

    for (const [index, item] of items.entries()) {
      this.updateItem(item, index)
    }
    this.rowOrder = nextOrder
  }

  setHideThinkingBlock(hide: boolean): void {
    if (this.hideThinkingBlock === hide) return
    this.hideThinkingBlock = hide
    for (const entry of this.rows.values()) {
      if (entry.kind === "row" && entry.view instanceof AssistantRowView) {
        entry.view.setHideThinkingBlock(hide)
      }
    }
  }

  destroy(): void {
    this.clearRows()
    this.root.destroyRecursively()
    this.syntaxStyle.destroy()
  }

  private createRow(row: TranscriptRow): TranscriptRowView {
    switch (row.kind) {
      case "assistant":
        return new AssistantRowView(
          this.ctx,
          this.syntaxStyle,
          this.options.treeSitterClient,
          row,
          this.hideThinkingBlock,
        )
      case "tool":
        return new ToolRowView(
          this.ctx,
          this.syntaxStyle,
          this.options.treeSitterClient,
          row,
        )
      case "user":
      case "notice":
      case "error":
        return new TextRowView(this.ctx, row)
    }
  }

  private updateItem(item: TranscriptDisplayItem, index: number): void {
    const entry = this.rows.get(item.id)
    if (item.kind === "read-group") {
      if (entry?.kind === "read-group") {
        const unchanged =
          entry.rows.length === item.rows.length &&
          entry.rows.every((row, rowIndex) => row === item.rows[rowIndex])
        if (unchanged) return
        entry.view.update(entry.rows, item.rows)
        entry.rows = item.rows
        return
      }
      entry?.view.destroy()
      const view = new ReadGroupRowView(
        this.ctx,
        this.syntaxStyle,
        this.options.treeSitterClient,
        item.rows,
      )
      this.rows.set(item.id, { kind: "read-group", view, rows: item.rows })
      this.root.add(view.root, index + 1)
      return
    }

    const row = item.row
    if (
      entry?.kind !== "row" ||
      entry.row.kind !== row.kind
    ) {
      entry?.view.destroy()
      const view = this.createRow(row)
      this.rows.set(item.id, { kind: "row", view, row })
      this.root.add(view.root, index + 1)
      return
    }
    entry.view.update(entry.row, row)
    entry.row = row
  }

  private clearRows(): void {
    for (const entry of this.rows.values()) entry.view.destroy()
    this.rows.clear()
    this.rowOrder = []
  }
}
