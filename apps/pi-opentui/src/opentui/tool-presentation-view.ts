import {
  BoxRenderable,
  CodeRenderable,
  DiffRenderable,
  MarkdownRenderable,
  pathToFiletype,
  SyntaxStyle,
  TextAttributes,
  TextRenderable,
  type RenderContext,
  type Renderable,
  type TreeSitterClient,
} from "@opentui/core"
import type {
  ToolPresentation,
  ToolPresentationBody,
  ToolRunState,
} from "../ui/tool-presentation.ts"
import type { OpenTuiComponent } from "./component.ts"
import { theme } from "./theme.ts"

export interface ToolPresentationViewOptions {
  readonly syntaxStyle?: SyntaxStyle
  readonly treeSitterClient?: TreeSitterClient
}

export const createToolSyntaxStyle = (): SyntaxStyle =>
  SyntaxStyle.fromStyles({
    default: { fg: theme.text },
    text: { fg: theme.text },
    keyword: { fg: theme.accent, bold: true },
    string: { fg: theme.assistant },
    number: { fg: theme.tool },
    boolean: { fg: theme.tool },
    comment: { fg: theme.muted, italic: true },
    function: { fg: theme.user },
    method: { fg: theme.user },
    type: { fg: theme.accent },
    variable: { fg: theme.text },
    property: { fg: theme.user },
    operator: { fg: theme.accent },
    punctuation: { fg: theme.muted },
    "markup.heading": { fg: theme.accent, bold: true },
    "markup.strong": { fg: theme.text, bold: true },
    "markup.italic": { fg: theme.text, italic: true },
    "markup.link": { fg: theme.user, underline: true },
    "markup.raw": { fg: theme.assistant },
    "markup.list": { fg: theme.accent },
    conceal: { fg: theme.muted },
  })

const statusText = (model: ToolPresentation): string => {
  const detail = model.detail === undefined ? "" : ` ${model.detail}`
  const state = model.showStateLabel === false
    ? ""
    : model.state === "pending"
      ? " …"
      : model.state === "error"
        ? " · failed"
        : ""
  return `${model.title}${detail}${state}`
}

const stateColor = (state: ToolRunState): string => {
  switch (state) {
    case "pending":
      return theme.accent
    case "complete":
      return theme.text
    case "error":
      return theme.error
  }
}

const statusColor = (model: ToolPresentation): string =>
  stateColor(model.state)

const readMarkerColor = (state: ToolRunState): string =>
  state === "complete" ? theme.assistant : stateColor(state)

const statusBackground = (model: ToolPresentation): string => {
  switch (model.state) {
    case "pending":
      return theme.toolPendingBg
    case "complete":
      return theme.toolSuccessBg
    case "error":
      return theme.toolErrorBg
  }
}

const filetypeFor = (
  body: Extract<ToolPresentationBody, { readonly kind: "code" | "diff" }>,
): string | undefined =>
  body.kind === "code" && body.language !== undefined
    ? body.language
    : body.path === undefined
      ? undefined
      : pathToFiletype(body.path)

export class ToolPresentationView
  implements OpenTuiComponent<ToolPresentation>
{
  readonly root: BoxRenderable

  private readonly header: TextRenderable
  private readonly syntaxStyle: SyntaxStyle
  private readonly ownsSyntaxStyle: boolean
  private body: Renderable | undefined
  private bodyModel: ToolPresentationBody | undefined
  private destroyed = false

  constructor(
    private readonly ctx: RenderContext,
    model: ToolPresentation,
    private readonly options: ToolPresentationViewOptions = {},
  ) {
    this.syntaxStyle = options.syntaxStyle ?? createToolSyntaxStyle()
    this.ownsSyntaxStyle = options.syntaxStyle === undefined
    this.root = new BoxRenderable(ctx, {
      width: "100%",
      flexDirection: "column",
      paddingLeft: 1,
      paddingRight: 1,
      paddingTop: 1,
      paddingBottom: 1,
      backgroundColor: statusBackground(model),
    })
    this.header = new TextRenderable(ctx, {
      content: "",
      attributes: TextAttributes.BOLD,
    })
    this.root.add(this.header)
    this.update(undefined, model)
  }

  update(
    previous: ToolPresentation | undefined,
    current: ToolPresentation,
  ): void {
    if (this.destroyed) return
    if (
      previous === undefined ||
      previous.title !== current.title ||
      previous.detail !== current.detail ||
      previous.state !== current.state ||
      previous.showStateLabel !== current.showStateLabel
    ) {
      this.header.content = statusText(current)
      this.header.fg = statusColor(current)
      this.root.backgroundColor = statusBackground(current)
    }
    this.updateBody(current.body)
  }

  destroy(): void {
    if (this.destroyed) return
    this.destroyed = true
    this.root.destroyRecursively()
    this.body = undefined
    this.bodyModel = undefined
    if (this.ownsSyntaxStyle) this.syntaxStyle.destroy()
  }

  private updateBody(current: ToolPresentationBody): void {
    const previous = this.bodyModel
    if (previous?.kind !== current.kind) {
      this.replaceBody(current)
      return
    }
    if (current.kind === "none") {
      this.bodyModel = current
      return
    }
    if (current.kind === "read-group") {
      this.replaceBody(current)
      return
    }
    if (this.body === undefined) {
      this.replaceBody(current)
      return
    }

    switch (current.kind) {
      case "text": {
        const oldBody = previous as Extract<
          ToolPresentationBody,
          { readonly kind: "text" }
        >
        const body = this.body as TextRenderable
        if (oldBody.content !== current.content) body.content = current.content
        break
      }
      case "code": {
        const oldBody = previous as Extract<
          ToolPresentationBody,
          { readonly kind: "code" }
        >
        const body = this.body as CodeRenderable
        if (oldBody.content !== current.content) body.content = current.content
        if (
          oldBody.path !== current.path ||
          oldBody.language !== current.language
        ) {
          body.filetype = filetypeFor(current)
        }
        if (oldBody.streaming !== current.streaming) {
          body.streaming = current.streaming
        }
        break
      }
      case "markdown": {
        const oldBody = previous as Extract<
          ToolPresentationBody,
          { readonly kind: "markdown" }
        >
        const body = this.body as MarkdownRenderable
        if (oldBody.content !== current.content) body.content = current.content
        if (oldBody.streaming !== current.streaming) {
          body.streaming = current.streaming
        }
        break
      }
      case "diff": {
        const oldBody = previous as Extract<
          ToolPresentationBody,
          { readonly kind: "diff" }
        >
        const body = this.body as DiffRenderable
        if (oldBody.patch !== current.patch) body.diff = current.patch
        if (oldBody.path !== current.path) body.filetype = filetypeFor(current)
        if (oldBody.showLineNumbers !== current.showLineNumbers) {
          body.showLineNumbers = current.showLineNumbers
        }
        break
      }
    }
    this.bodyModel = current
  }

  private replaceBody(model: ToolPresentationBody): void {
    this.body?.destroyRecursively()
    this.body = this.createBody(model)
    this.bodyModel = model
    if (this.body !== undefined) this.root.add(this.body)
  }

  private createBody(model: ToolPresentationBody): Renderable | undefined {
    switch (model.kind) {
      case "none":
        return undefined
      case "text":
        return new TextRenderable(this.ctx, {
          content: model.content,
          fg: theme.muted,
          wrapMode: "word",
        })
      case "code": {
        const filetype = filetypeFor(model)
        return new CodeRenderable(this.ctx, {
          content: model.content,
          ...(filetype === undefined ? {} : { filetype }),
          syntaxStyle: this.syntaxStyle,
          ...(this.options.treeSitterClient === undefined
            ? {}
            : { treeSitterClient: this.options.treeSitterClient }),
          drawUnstyledText: true,
          conceal: false,
          streaming: model.streaming,
          wrapMode: "word",
        })
      }
      case "markdown":
        return new MarkdownRenderable(this.ctx, {
          content: model.content,
          syntaxStyle: this.syntaxStyle,
          ...(this.options.treeSitterClient === undefined
            ? {}
            : { treeSitterClient: this.options.treeSitterClient }),
          fg: theme.text,
          conceal: true,
          concealCode: false,
          streaming: model.streaming,
        })
      case "diff": {
        const filetype = filetypeFor(model)
        return new DiffRenderable(this.ctx, {
          diff: model.patch,
          view: "unified",
          ...(filetype === undefined ? {} : { filetype }),
          syntaxStyle: this.syntaxStyle,
          ...(this.options.treeSitterClient === undefined
            ? {}
            : { treeSitterClient: this.options.treeSitterClient }),
          wrapMode: "word",
          conceal: false,
          showLineNumbers: model.showLineNumbers,
          fg: theme.text,
          addedBg: "#233523",
          removedBg: "#3c2424",
          addedSignColor: theme.assistant,
          removedSignColor: theme.error,
        })
      }
      case "read-group": {
        const group = new BoxRenderable(this.ctx, {
          flexDirection: "column",
        })
        for (const entry of model.entries) {
          const row = new BoxRenderable(this.ctx, {
            flexDirection: "row",
          })
          row.add(new TextRenderable(this.ctx, {
            content: entry.marker,
            fg: readMarkerColor(entry.state),
            attributes: TextAttributes.BOLD,
          }))
          row.add(new TextRenderable(this.ctx, {
            content: ` ${entry.path}`,
            fg: theme.user,
          }))
          row.add(new TextRenderable(this.ctx, {
            content: `  ${entry.label}`,
            fg: theme.muted,
          }))
          group.add(row)
        }
        return group
      }
    }
  }
}
