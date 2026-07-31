import {
  BoxRenderable,
  SelectRenderable,
  SelectRenderableEvents,
  TextareaRenderable,
  TextAttributes,
  TextRenderable,
  type KeyEvent,
  type PasteEvent,
  type RenderContext,
  type SelectOption,
} from "@opentui/core"
import type { CommandInfo } from "../services/commands.ts"
import type { AppDialog } from "../services/extension-ui.ts"
import {
  commandMenuOptions,
  commandNameFromMenuOption,
} from "../ui/app-view-model.ts"
import {
  SearchMenuView,
  type SearchMenuProps,
} from "./search-menu-view.ts"
import { theme } from "./theme.ts"

export interface OverlayView {
  readonly root: BoxRenderable
  focus(): void
  cancel(): void
  destroy(): void
}

class ExtensionDialogView implements OverlayView {
  readonly root: BoxRenderable

  private readonly input: TextareaRenderable | undefined
  private readonly select: SelectRenderable | undefined
  private resolved = false
  private secret = ""
  private secretCursor = 0

  constructor(
    ctx: RenderContext,
    private readonly dialog: Exclude<AppDialog, { readonly kind: "search" }>,
    private readonly resolve: (value: string | undefined) => void,
  ) {
    this.root = new BoxRenderable(ctx, {
      position: "absolute",
      left: "10%",
      top: "20%",
      width: "80%",
      minHeight: 8,
      zIndex: 20,
      flexDirection: "column",
      border: true,
      borderColor: theme.accent,
      backgroundColor: theme.panel,
      paddingLeft: 2,
      paddingRight: 2,
      paddingTop: 1,
      paddingBottom: 1,
      gap: 1,
    })
    this.root.add(
      new TextRenderable(ctx, {
        content: dialog.title,
        fg: theme.accent,
        attributes: TextAttributes.BOLD,
        wrapMode: "word",
      }),
    )
    this.root.add(
      new TextRenderable(ctx, {
        content: dialog.message ?? "",
        fg: theme.text,
        wrapMode: "word",
      }),
    )

    if (dialog.kind === "select") {
      this.select = new SelectRenderable(ctx, {
        height: Math.max(1, Math.min(8, dialog.options.length)),
        options: dialog.options.map((option) => ({
          name: option,
          description: "",
          value: option,
        })),
        showDescription: false,
        backgroundColor: theme.panel,
        textColor: theme.text,
        focusedBackgroundColor: theme.panel,
        focusedTextColor: theme.text,
        selectedBackgroundColor: theme.border,
        selectedTextColor: theme.accent,
      })
      this.select.on(
        SelectRenderableEvents.ITEM_SELECTED,
        (_index: number, option: SelectOption | null) => {
          this.finish(
            typeof option?.value === "string" ? option.value : undefined,
          )
        },
      )
      this.root.add(this.select)
      this.input = undefined
    } else {
      const inputColor = dialog.kind === "secret" ? theme.muted : theme.text
      this.input = new TextareaRenderable(ctx, {
        height: 2,
        placeholder: dialog.placeholder ?? "Type a response",
        placeholderColor: theme.muted,
        textColor: inputColor,
        focusedTextColor: inputColor,
        backgroundColor: theme.background,
        focusedBackgroundColor: theme.background,
        cursorColor: theme.accent,
        wrapMode: "word",
        keyBindings: [
          { name: "return", action: "submit" },
          { name: "kpenter", action: "submit" },
          { name: "return", shift: true, action: "newline" },
        ],
        onSubmit: () => this.submitInput(),
        ...(dialog.kind === "secret"
          ? {
              onKeyDown: (event: KeyEvent) => this.handleSecretKey(event),
              onPaste: (event: PasteEvent) => this.handleSecretPaste(event),
            }
          : {}),
      })
      this.root.add(this.input)
      this.select = undefined
    }

    this.root.add(
      new TextRenderable(ctx, {
        content:
          dialog.kind === "select"
            ? "↑/↓ choose · enter confirm · esc cancel"
            : dialog.kind === "secret"
              ? "input hidden · enter confirm · esc cancel"
              : "enter confirm · shift+enter newline · esc cancel",
        fg: theme.muted,
      }),
    )
  }

  focus(): void {
    this.select?.focus()
    this.input?.focus()
  }

  cancel(): void {
    this.finish(undefined)
  }

  destroy(): void {
    this.root.destroyRecursively()
  }

  private finish(value: string | undefined): void {
    if (this.resolved) return
    this.resolved = true
    this.resolve(value)
  }

  private submitInput(): void {
    const value =
      this.dialog.kind === "secret"
        ? this.secret.trim()
        : this.input?.plainText.trim()
    this.finish(value && value.length > 0 ? value : undefined)
  }

  private withSecretBuffer(
    edit: (renderable: TextareaRenderable) => void,
  ): void {
    if (this.input === undefined) return
    this.input.editBuffer.setText(this.secret)
    this.input.cursorOffset = this.secretCursor
    edit(this.input)
    this.secret = this.input.plainText
    this.secretCursor = this.input.cursorOffset
    this.input.editBuffer.setText("•".repeat(this.secret.length))
    this.input.cursorOffset = this.secretCursor
  }

  private handleSecretKey(event: KeyEvent): void {
    event.preventDefault()
    event.stopPropagation()
    this.withSecretBuffer((renderable) => renderable.handleKeyPress(event))
  }

  private handleSecretPaste(event: PasteEvent): void {
    event.preventDefault()
    event.stopPropagation()
    this.withSecretBuffer((renderable) => renderable.handlePaste(event))
  }
}
export const createDialogView = (
  ctx: RenderContext,
  dialog: AppDialog,
  resolve: (value: string | undefined) => void,
): OverlayView => {
  if (dialog.kind === "search") {
    const props: SearchMenuProps = {
      title: dialog.title,
      options: dialog.options,
      resolve,
      ...(dialog.message === undefined ? {} : { message: dialog.message }),
      ...(dialog.initialQuery === undefined
        ? {}
        : { initialQuery: dialog.initialQuery }),
    }
    return new SearchMenuView(ctx, props)
  }
  return new ExtensionDialogView(ctx, dialog, resolve)
}

export const createCommandMenu = (
  ctx: RenderContext,
  commands: ReadonlyArray<CommandInfo>,
  resolve: (name: string | undefined) => void,
): SearchMenuView =>
  new SearchMenuView(ctx, {
    title: "Commands",
    message: "Choose a command to run.",
    options: commandMenuOptions(commands),
    resolve: (selected) => resolve(commandNameFromMenuOption(selected)),
  })

export class AuthNoticeView {
  readonly root: BoxRenderable

  constructor(ctx: RenderContext, message: string) {
    this.root = new BoxRenderable(ctx, {
      position: "absolute",
      left: "10%",
      top: "20%",
      width: "80%",
      minHeight: 7,
      zIndex: 15,
      flexDirection: "column",
      border: true,
      borderColor: theme.accent,
      backgroundColor: theme.panel,
      paddingLeft: 2,
      paddingRight: 2,
      paddingTop: 1,
      paddingBottom: 1,
      gap: 1,
    })
    this.root.add(
      new TextRenderable(ctx, {
        content: "Login",
        fg: theme.accent,
        attributes: TextAttributes.BOLD,
      }),
    )
    this.root.add(
      new TextRenderable(ctx, {
        content: message,
        fg: theme.text,
        wrapMode: "word",
      }),
    )
    this.root.add(
      new TextRenderable(ctx, {
        content: "esc cancel",
        fg: theme.muted,
      }),
    )
  }

  destroy(): void {
    this.root.destroyRecursively()
  }
}
