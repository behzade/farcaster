import {
  BoxRenderable,
  MarkdownRenderable,
  TextRenderable,
  type RenderContext,
  type Renderable,
  type SyntaxStyle,
  type TreeSitterClient,
} from "@opentui/core"
import type { OpenTuiComponent } from "./component.ts"

export interface StableMarkdownModel {
  readonly content: string
  readonly streaming: boolean
  readonly color: string
}

export interface StableMarkdownViewOptions {
  readonly syntaxStyle: SyntaxStyle
  readonly treeSitterClient?: TreeSitterClient
}

/**
 * Keeps streamed text on one cheap render path. OpenTUI Markdown can expose
 * its raw fallback again each time an async parse restarts; switching only
 * when the stream settles avoids that raw/formatted flash cycle.
 */
export class StableMarkdownView
  implements OpenTuiComponent<StableMarkdownModel>
{
  readonly root: BoxRenderable

  private child: TextRenderable | MarkdownRenderable
  private mode: "streaming" | "settled"
  private destroyed = false

  constructor(
    private readonly ctx: RenderContext,
    model: StableMarkdownModel,
    private readonly options: StableMarkdownViewOptions,
  ) {
    this.root = new BoxRenderable(ctx, {
      width: "100%",
      flexDirection: "column",
    })
    this.mode = model.streaming ? "streaming" : "settled"
    this.child = this.createChild(model)
    this.root.add(this.child)
  }

  update(
    previous: StableMarkdownModel | undefined,
    current: StableMarkdownModel,
  ): void {
    if (this.destroyed) return
    const nextMode = current.streaming ? "streaming" : "settled"
    if (nextMode !== this.mode) {
      this.child.destroyRecursively()
      this.mode = nextMode
      this.child = this.createChild(current)
      this.root.add(this.child)
      return
    }

    if (previous?.content !== current.content) {
      this.child.content = current.content
    }
    if (previous?.color !== current.color) this.child.fg = current.color
  }

  destroy(): void {
    if (this.destroyed) return
    this.destroyed = true
    this.root.destroyRecursively()
  }

  currentRenderable(): Renderable {
    return this.child
  }

  private createChild(
    model: StableMarkdownModel,
  ): TextRenderable | MarkdownRenderable {
    if (model.streaming) {
      return new TextRenderable(this.ctx, {
        content: model.content,
        fg: model.color,
        wrapMode: "word",
      })
    }
    return new MarkdownRenderable(this.ctx, {
      content: model.content,
      syntaxStyle: this.options.syntaxStyle,
      ...(this.options.treeSitterClient === undefined
        ? {}
        : { treeSitterClient: this.options.treeSitterClient }),
      fg: model.color,
      conceal: true,
      concealCode: false,
      streaming: false,
    })
  }
}
