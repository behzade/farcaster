import {
  BoxRenderable,
  ScrollBoxRenderable,
  TextAttributes,
  TextRenderable,
  type CliRenderer,
  type KeyEvent,
  type RenderContext,
} from "@opentui/core"
import type { AppSnapshot } from "../services/app-state-model.ts"
import {
  canInterrupt,
  showsAgentWorking,
} from "../services/app-activity.ts"
import type { AppClient } from "../ui/app-client.ts"
import type { KeybindingsShape } from "../services/keybindings.ts"
import { headerViewModel } from "../ui/app-view-model.ts"
import { ComposerView } from "./composer-view.ts"
import type { OpenTuiComponent } from "./component.ts"
import {
  AuthNoticeView,
  createDialogView,
  type OverlayView,
} from "./dialog-view.ts"
import { theme } from "./theme.ts"
import { TranscriptView } from "./transcript-view.ts"

export class AppView implements OpenTuiComponent<AppSnapshot> {
  readonly root: BoxRenderable

  private readonly activity: TextRenderable
  private readonly usage: TextRenderable
  private readonly location: TextRenderable
  private readonly transcript: TranscriptView
  private readonly composer: ComposerView
  private snapshot: AppSnapshot
  private dialog: OverlayView | undefined
  private authNotice: AuthNoticeView | undefined
  private unsubscribe: (() => void) | undefined
  private mounted = false
  private destroyed = false
  private lastClearTime = 0

  private readonly handleKey = (event: KeyEvent): void => {
    if (
      this.keybindings.matches(event.raw, "tui.select.cancel") &&
      this.dialog !== undefined
    ) {
      event.preventDefault()
      event.stopPropagation()
      this.dialog.cancel()
      return
    }
    if (
      this.keybindings.matches(event.raw, "tui.select.cancel") &&
      this.composer.cancelPalette(event)
    ) {
      return
    }
    if (this.keybindings.matches(event.raw, "app.clear")) {
      event.preventDefault()
      event.stopPropagation()
      const now = Date.now()
      this.composer.clearDraft()
      if (now - this.lastClearTime < 500) this.client.quit()
      this.lastClearTime = now
      return
    }
    if (
      this.keybindings.matches(event.raw, "app.exit") &&
      this.composer.isDraftEmpty()
    ) {
      event.preventDefault()
      event.stopPropagation()
      this.client.quit()
      return
    }
    if (
      this.keybindings.matches(event.raw, "app.interrupt") &&
      canInterrupt(this.snapshot.activity)
    ) {
      event.preventDefault()
      event.stopPropagation()
      this.client.dispatch({ _tag: "Abort" })
    }
  }

  constructor(
    private readonly renderer: CliRenderer,
    private readonly client: AppClient,
    private readonly keybindings: KeybindingsShape,
  ) {
    const ctx: RenderContext = renderer
    this.snapshot = client.initial
    this.root = new BoxRenderable(ctx, {
      width: "100%",
      height: "100%",
      flexDirection: "column",
      paddingLeft: 1,
      paddingRight: 1,
      backgroundColor: theme.background,
    })

    const header = new BoxRenderable(ctx, {
      height: 1,
      flexDirection: "row",
      justifyContent: "space-between",
    })
    const title = new BoxRenderable(ctx, { flexDirection: "row" })
    title.add(
      new TextRenderable(ctx, {
        content: "pi",
        fg: theme.accent,
        attributes: TextAttributes.BOLD,
      }),
    )
    this.activity = new TextRenderable(ctx, {
      content: "",
      fg: theme.muted,
    })
    title.add(this.activity)
    header.add(title)
    this.usage = new TextRenderable(ctx, {
      content: "",
      fg: "#a89984",
    })
    header.add(this.usage)
    this.root.add(header)

    this.location = new TextRenderable(ctx, {
      content: "",
      fg: theme.muted,
      wrapMode: "none",
    })
    this.root.add(this.location)

    const scroll = new ScrollBoxRenderable(ctx, {
      flexGrow: 1,
      stickyScroll: true,
      stickyStart: "bottom",
      viewportCulling: true,
      scrollY: true,
    })
    this.transcript = new TranscriptView(ctx, {
      hideThinkingBlock: client.initial.hideThinkingBlock,
    })
    scroll.add(this.transcript.root)
    this.root.add(scroll)

    this.composer = new ComposerView(ctx, {
      snapshot: client.initial,
      projectPaths: client.projectPaths,
      dispatch: client.dispatch,
      resolvePaste: client.resolvePaste,
      keybindings,
      overlayParent: this.root,
    })
    this.root.add(this.composer.root)
  }

  mount(): void {
    if (this.mounted || this.destroyed) return
    this.mounted = true
    this.renderer.root.add(this.root)
    this.renderer.keyInput.on("keypress", this.handleKey)
    this.update(undefined, this.client.initial)
    this.unsubscribe = this.client.subscribe((snapshot) => {
      this.update(this.snapshot, snapshot)
    })
    this.composer.focusIfReady()
  }

  update(previous: AppSnapshot | undefined, current: AppSnapshot): void {
    if (this.destroyed) return
    this.snapshot = current
    if (
      previous?.terminalTitle !== current.terminalTitle ||
      previous?.cwd !== current.cwd
    ) {
      const cwdName = current.cwd.split("/").filter(Boolean).at(-1) ?? current.cwd
      this.renderer.setTerminalTitle(
        current.terminalTitle ?? `π · ${cwdName}`,
      )
    }
    const oldHeader = previous === undefined ? undefined : headerViewModel(previous)
    const nextHeader = headerViewModel(current)
    if (oldHeader?.activity !== nextHeader.activity) {
      this.activity.content = ` · ${nextHeader.activity}`
    }
    if (oldHeader?.usage !== nextHeader.usage) {
      this.usage.content = nextHeader.usage
    }
    if (oldHeader?.location !== nextHeader.location) {
      this.location.content = nextHeader.location
    }

    if (
      previous === undefined ||
      previous.hideThinkingBlock !== current.hideThinkingBlock
    ) {
      this.transcript.setHideThinkingBlock(current.hideThinkingBlock)
    }
    this.transcript.setWorking(showsAgentWorking(current.activity))
    this.transcript.update(previous?.transcript, current.transcript)
    this.updateDialog(previous, current)
    this.updateAuthNotice(previous, current)
    this.composer.update(previous, current)
  }

  destroy(): void {
    if (this.destroyed) return
    this.destroyed = true
    this.unsubscribe?.()
    this.unsubscribe = undefined
    this.renderer.keyInput.off("keypress", this.handleKey)
    this.dialog?.destroy()
    this.dialog = undefined
    this.authNotice?.destroy()
    this.authNotice = undefined
    this.composer.destroy()
    this.transcript.destroy()
    this.root.destroyRecursively()
  }

  private updateDialog(
    previous: AppSnapshot | undefined,
    current: AppSnapshot,
  ): void {
    if (previous?.dialog === current.dialog) return
    this.dialog?.destroy()
    this.dialog = undefined
    if (current.dialog === undefined) return

    const dialog = current.dialog
    this.dialog = createDialogView(this.renderer, dialog, (value) => {
      this.client.dispatch({
        _tag: "ResolveDialog",
        id: dialog.id,
        value,
      })
    })
    this.root.add(this.dialog.root)
    this.dialog.focus()
  }

  private updateAuthNotice(
    previous: AppSnapshot | undefined,
    current: AppSnapshot,
  ): void {
    if (previous?.authNotice === current.authNotice) return
    this.authNotice?.destroy()
    this.authNotice = undefined
    if (current.authNotice === undefined) return

    this.authNotice = new AuthNoticeView(this.renderer, current.authNotice)
    this.root.add(this.authNotice.root)
  }
}

export const mountApp = (
  renderer: CliRenderer,
  client: AppClient,
  keybindings: KeybindingsShape,
): AppView => {
  const view = new AppView(renderer, client, keybindings)
  view.mount()
  return view
}
