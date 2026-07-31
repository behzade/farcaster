import {
  BoxRenderable,
  TextareaRenderable,
  TextRenderable,
  type KeyEvent,
  type PasteEvent,
  type RenderContext,
} from "@opentui/core"
import type {
  AppCommand,
  AppSnapshot,
} from "../services/app-state-model.ts"
import {
  type CommandInfo,
  exactSlashCommand,
  selectSlashCommand,
  slashCommandMatches,
} from "../services/commands.ts"
import {
  applyFileMentionCompletion,
  fileMentionMatches,
  type FileCompletion,
} from "../services/file-completion.ts"
import type { ProjectPath } from "../services/project-paths.ts"
import {
  primaryKey,
  type KeybindingsShape,
} from "../services/keybindings.ts"
import {
  isLargePaste,
  type PasteInsertion,
  type PasteRequest,
} from "../services/paste-model.ts"
import type { OpenTuiComponent } from "./component.ts"
import { createCommandMenu } from "./dialog-view.ts"
import type { SearchMenuView } from "./search-menu-view.ts"
import { theme } from "./theme.ts"

export interface ComposerOptions {
  readonly snapshot: AppSnapshot
  readonly projectPaths: () => ReadonlyArray<ProjectPath>
  readonly dispatch: (command: AppCommand) => void
  readonly resolvePaste: (
    request: PasteRequest,
    accept: (insertion: PasteInsertion | undefined) => void,
  ) => void
  readonly keybindings: KeybindingsShape
  readonly overlayParent: BoxRenderable
}

export class ComposerView implements OpenTuiComponent<AppSnapshot> {
  readonly root: BoxRenderable

  private readonly input: TextareaRenderable
  private readonly hint: TextRenderable
  private snapshot: AppSnapshot
  private palette: SearchMenuView | undefined
  private suggestionIndex = 0
  private cachedFileText = ""
  private cachedFileCursor = -1
  private cachedProjectPaths: ReadonlyArray<ProjectPath> | undefined
  private cachedFileSuggestions: ReadonlyArray<FileCompletion> = []
  private readonly pendingPastes = new Map<number, string>()
  private nextPasteId = 1
  private destroyed = false

  constructor(
    private readonly ctx: RenderContext,
    private readonly options: ComposerOptions,
  ) {
    this.snapshot = options.snapshot
    this.root = new BoxRenderable(ctx, {
      height: 4,
      flexDirection: "column",
      border: ["top"],
      borderColor: theme.border,
      paddingLeft: 1,
      paddingRight: 1,
    })
    this.input = new TextareaRenderable(ctx, {
      height: 2,
      placeholder: this.placeholder(),
      placeholderColor: theme.muted,
      textColor: theme.text,
      focusedTextColor: theme.text,
      backgroundColor: theme.background,
      focusedBackgroundColor: theme.background,
      cursorColor: theme.accent,
      wrapMode: "word",
      scrollMargin: 0,
      keyBindings: [
        { name: "return", action: "submit" },
        { name: "kpenter", action: "submit" },
        { name: "return", shift: true, action: "newline" },
        { name: "kpenter", shift: true, action: "newline" },
      ],
      onKeyDown: (event) => this.handleKey(event),
      onPaste: (event) => this.handlePaste(event),
      onSubmit: () => this.submit(),
    })
    this.root.add(this.input)

    const footer = new BoxRenderable(ctx, {
      height: 1,
      flexDirection: "row",
      justifyContent: "space-between",
    })
    this.hint = new TextRenderable(ctx, {
      content: this.defaultHint(),
      fg: theme.muted,
    })
    footer.add(this.hint)
    footer.add(
      new TextRenderable(ctx, {
        content: `${primaryKey(options.keybindings, "app.exit")} quit`,
        fg: theme.muted,
      }),
    )
    this.root.add(footer)
  }

  update(previous: AppSnapshot | undefined, current: AppSnapshot): void {
    this.snapshot = current
    if (previous?.phase !== current.phase) {
      this.input.placeholder = this.placeholder()
    }
    if (current.dialog !== undefined && this.palette !== undefined) {
      this.destroyPalette(false)
    }
    if (
      previous === undefined ||
      previous.phase !== current.phase ||
      previous.dialog !== current.dialog ||
      previous.commands !== current.commands
    ) {
      this.updateHint()
    }
    if (previous === undefined || previous.dialog !== current.dialog) {
      this.focusIfReady()
    }
  }

  focusIfReady(): void {
    if (
      this.snapshot.dialog === undefined &&
      this.palette === undefined &&
      this.snapshot.phase !== "fatal"
    ) {
      this.input.focus()
    } else {
      this.input.blur()
    }
  }

  isDraftEmpty(): boolean {
    return this.input.plainText.length === 0
  }

  clearDraft(): void {
    this.setDraft("")
  }

  cancelPalette(event?: KeyEvent): boolean {
    if (this.palette === undefined) return false
    event?.preventDefault()
    event?.stopPropagation()
    this.palette.cancel()
    return true
  }

  destroy(): void {
    this.destroyed = true
    this.destroyPalette(false)
    this.root.destroyRecursively()
  }

  private placeholder(): string {
    return this.snapshot.phase === "fatal"
      ? "Pi event stream failed"
      : this.snapshot.phase === "running"
        ? "Pi is working…"
        : "Ask Pi"
  }

  private canComplete(): boolean {
    return (
      this.snapshot.dialog === undefined &&
      this.palette === undefined &&
      this.snapshot.phase !== "running" &&
      this.snapshot.phase !== "stopping" &&
      this.snapshot.phase !== "fatal"
    )
  }

  private commandSuggestions(): ReadonlyArray<CommandInfo> {
    return slashCommandMatches(this.snapshot.commands, this.input.plainText)
  }

  private fileSuggestions(): ReadonlyArray<FileCompletion> {
    const text = this.input.plainText
    const cursor = this.input.cursorOffset
    const paths = this.options.projectPaths()
    if (
      text !== this.cachedFileText ||
      cursor !== this.cachedFileCursor ||
      paths !== this.cachedProjectPaths
    ) {
      this.cachedFileText = text
      this.cachedFileCursor = cursor
      this.cachedProjectPaths = paths
      this.cachedFileSuggestions = fileMentionMatches(paths, text, cursor)
    }
    return this.cachedFileSuggestions
  }

  private activeCount(): number {
    if (!this.canComplete()) return 0
    const files = this.fileSuggestions()
    return files.length > 0 ? files.length : this.commandSuggestions().length
  }

  private selectedIndex(length: number): number {
    return Math.min(this.suggestionIndex, length - 1)
  }

  private updateHint(): void {
    if (this.pendingPastes.size > 0) {
      this.hint.content = `${this.pendingPastes.size} paste${this.pendingPastes.size === 1 ? "" : "s"} loading…`
      return
    }
    const files = this.fileSuggestions()
    const file = files[this.selectedIndex(files.length)]
    if (this.canComplete() && file !== undefined) {
      this.hint.content = `${file.path}${file.isDirectory ? " · folder" : ""} · ↑/↓ choose · tab or enter complete`
      return
    }

    const commands = this.commandSuggestions()
    const command = commands[this.selectedIndex(commands.length)]
    this.hint.content =
      this.snapshot.phase === "fatal"
        ? "restart pi-next after updating the Pi event handler"
        : this.canComplete() && command !== undefined
        ? `/${command.name}${command.description.length > 0 ? ` — ${command.description}` : ""} · ↑/↓ choose · tab or enter complete`
        : this.defaultHint()
  }

  private defaultHint(): string {
    return `enter send · shift+enter newline · ${primaryKey(this.options.keybindings, "app.clipboard.pasteImage")} paste · / commands · @ files`
  }

  private setDraft(text: string, cursorOffset = text.length): void {
    this.input.editBuffer.setText(text)
    this.input.cursorOffset = cursorOffset
    this.input.focus()
    this.suggestionIndex = 0
    this.updateHint()
  }

  private completeCommand(command: CommandInfo): void {
    this.setDraft(`/${command.name} `)
  }

  private completeFile(completion: FileCompletion): void {
    const result = applyFileMentionCompletion(
      this.input.plainText,
      this.input.cursorOffset,
      completion,
    )
    if (result !== undefined) {
      this.setDraft(result.text, result.cursorOffset)
    }
  }

  private completeSelected(): boolean {
    if (!this.canComplete()) return false
    const files = this.fileSuggestions()
    const file = files[this.selectedIndex(files.length)]
    if (file !== undefined) {
      this.completeFile(file)
      return true
    }
    const commands = this.commandSuggestions()
    const command = commands[this.selectedIndex(commands.length)]
    if (command !== undefined) {
      this.completeCommand(command)
      return true
    }
    return false
  }

  private openPalette(): void {
    if (this.palette !== undefined) return
    this.palette = createCommandMenu(
      this.ctx,
      this.snapshot.commands,
      (name) => {
        this.destroyPalette(false)
        if (name !== undefined && name.length > 0) {
          this.setDraft(`/${name} `)
        } else {
          this.focusIfReady()
          this.updateHint()
        }
      },
    )
    this.options.overlayParent.add(this.palette.root)
    this.palette.focus()
  }

  private destroyPalette(refocus: boolean): void {
    const palette = this.palette
    this.palette = undefined
    palette?.destroy()
    if (refocus) this.focusIfReady()
  }

  private submit(): void {
    if (this.pendingPastes.size > 0) {
      this.updateHint()
      return
    }
    const prompt = this.input.plainText.trim()
    if (prompt === "/" && this.snapshot.phase === "ready") {
      this.openPalette()
      return
    }
    if (this.fileSuggestions().length > 0 && this.completeSelected()) return
    if (
      prompt.startsWith("/") &&
      exactSlashCommand(this.snapshot.commands, prompt) === undefined &&
      this.completeSelected()
    ) {
      return
    }
    if (
      prompt.length === 0 ||
      this.snapshot.phase === "running" ||
      this.snapshot.phase === "stopping" ||
      this.snapshot.phase === "fatal"
    ) {
      return
    }

    const command = selectSlashCommand(this.snapshot.commands, prompt)
    this.options.dispatch(
      command === undefined
        ? { _tag: "Prompt", text: prompt }
        : { _tag: "RunCommand", ...command },
    )
    this.setDraft("")
  }

  private handleKey(event: KeyEvent): void {
    const pasteClipboard = this.options.keybindings.matches(
      event.raw,
      "app.clipboard.pasteImage",
    )
    if (pasteClipboard) {
      event.preventDefault()
      event.stopPropagation()
      if (this.snapshot.phase !== "fatal") {
        this.requestPaste({ kind: "clipboard" })
      }
      return
    }
    const count = this.activeCount()
    if (
      count > 0 &&
      (event.name === "tab" || event.name === "up" || event.name === "down")
    ) {
      event.preventDefault()
      if (event.name === "tab") {
        this.completeSelected()
      } else {
        const offset = event.name === "up" ? -1 : 1
        this.suggestionIndex =
          (this.suggestionIndex + offset + count) % count
        this.updateHint()
      }
      return
    }
    if (
      this.options.keybindings.matches(event.raw, "app.interrupt") &&
      (this.snapshot.phase === "running" ||
        this.snapshot.phase === "stopping")
    ) {
      return
    }

    event.preventDefault()
    this.input.handleKeyPress(event)
    this.suggestionIndex = 0
    this.updateHint()
  }

  private handlePaste(event: PasteEvent): void {
    event.preventDefault()
    if (this.snapshot.phase === "fatal") return
    const text = new TextDecoder().decode(event.bytes)
    if (isLargePaste(text)) {
      this.requestPaste({ kind: "text", text })
      return
    }
    this.input.handlePaste(event)
    this.suggestionIndex = 0
    this.updateHint()
  }

  private requestPaste(request: PasteRequest): void {
    const id = this.nextPasteId
    this.nextPasteId += 1
    const marker = `[paste #${id} ${crypto.randomUUID().slice(0, 8)} loading]`
    this.pendingPastes.set(id, marker)
    this.input.insertText(marker)
    this.suggestionIndex = 0
    this.updateHint()
    this.options.resolvePaste(request, (insertion) =>
      this.finishPaste(id, insertion),
    )
  }

  private finishPaste(
    id: number,
    insertion: PasteInsertion | undefined,
  ): void {
    if (this.destroyed || this.snapshot.phase === "fatal") return
    const marker = this.pendingPastes.get(id)
    if (marker === undefined) return
    this.pendingPastes.delete(id)
    const current = this.input.plainText
    const start = current.indexOf(marker)
    if (start < 0) {
      this.updateHint()
      return
    }

    let text = insertion?.text ?? ""
    if (insertion?.kind === "file") {
      const before = current.slice(0, start)
      const after = current.slice(start + marker.length)
      if (before.length > 0 && !/\s$/.test(before)) text = ` ${text}`
      if (after.length > 0 && !/^\s/.test(after)) text = `${text} `
    }
    const cursor = this.input.cursorOffset
    const markerEnd = start + marker.length
    const next = `${current.slice(0, start)}${text}${current.slice(markerEnd)}`
    const nextCursor =
      cursor <= start
        ? cursor
        : cursor >= markerEnd
          ? cursor + text.length - marker.length
          : start + text.length
    this.input.editBuffer.setText(next)
    this.input.cursorOffset = nextCursor
    this.suggestionIndex = 0
    this.updateHint()
  }
}
