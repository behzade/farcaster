import {
  BoxRenderable,
  InputRenderable,
  SelectRenderable,
  SelectRenderableEvents,
  TextRenderable,
  TextAttributes,
  type KeyEvent,
  type PasteEvent,
  type RenderContext,
  type SelectOption,
} from "@opentui/core"
import {
  filterOptions,
  searchResultLimit,
} from "../ui/search.ts"
import { theme } from "./theme.ts"

export interface SearchMenuProps {
  readonly title: string
  readonly message?: string
  readonly options: ReadonlyArray<string>
  readonly initialQuery?: string
  readonly resolve: (value: string | undefined) => void
}

const selectOptions = (
  options: ReadonlyArray<string>,
): Array<SelectOption> =>
  options.map((option) => ({
    name: option,
    description: "",
    value: option,
  }))

export class SearchMenuView {
  readonly root: BoxRenderable

  private readonly input: InputRenderable
  private readonly select: SelectRenderable
  private readonly empty: TextRenderable
  private resolved = false

  constructor(
    ctx: RenderContext,
    private readonly props: SearchMenuProps,
  ) {
    const initialOptions = filterOptions(
      props.options,
      props.initialQuery ?? "",
    )

    this.root = new BoxRenderable(ctx, {
      position: "absolute",
      left: "10%",
      top: "15%",
      width: "80%",
      minHeight: 9,
      zIndex: 30,
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
        content: props.title,
        fg: theme.accent,
        attributes: TextAttributes.BOLD,
        wrapMode: "word",
      }),
    )
    this.root.add(
      new TextRenderable(ctx, {
        content: props.message ?? "",
        fg: theme.text,
        wrapMode: "word",
      }),
    )

    this.input = new InputRenderable(ctx, {
      value: props.initialQuery ?? "",
      placeholder: "Type to search",
      placeholderColor: theme.muted,
      textColor: theme.text,
      focusedTextColor: theme.text,
      backgroundColor: theme.background,
      focusedBackgroundColor: theme.background,
      cursorColor: theme.accent,
      onKeyDown: (event) => this.handleKey(event),
      onPaste: (event) => this.handlePaste(event),
    })
    this.root.add(this.input)

    this.select = new SelectRenderable(ctx, {
      height: searchResultLimit,
      options: selectOptions(initialOptions),
      showDescription: false,
      wrapSelection: true,
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

    this.empty = new TextRenderable(ctx, {
      content: initialOptions.length === 0 ? "No matches" : "",
      fg: theme.muted,
    })
    this.root.add(this.empty)
    this.root.add(
      new TextRenderable(ctx, {
        content: "↑/↓ choose · enter confirm · esc cancel",
        fg: theme.muted,
      }),
    )
  }

  focus(): void {
    this.input.focus()
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
    this.props.resolve(value)
  }

  private currentOptions(): ReadonlyArray<string> {
    return filterOptions(this.props.options, this.input.value)
  }

  private refresh(): void {
    const options = this.currentOptions()
    this.select.options = selectOptions(options)
    this.select.setSelectedIndex(0)
    this.empty.content = options.length === 0 ? "No matches" : ""
  }

  private handleKey(event: KeyEvent): void {
    if (event.name === "escape") {
      event.preventDefault()
      event.stopPropagation()
      this.cancel()
      return
    }
    if (event.name === "up" || event.name === "down") {
      event.preventDefault()
      event.stopPropagation()
      if (event.name === "up") this.select.moveUp()
      else this.select.moveDown()
      return
    }
    if (event.name === "return" || event.name === "kpenter") {
      event.preventDefault()
      event.stopPropagation()
      const value = this.select.getSelectedOption()?.value
      if (typeof value === "string") this.finish(value)
      return
    }

    event.preventDefault()
    event.stopPropagation()
    this.input.handleKeyPress(event)
    this.refresh()
  }

  private handlePaste(event: PasteEvent): void {
    event.preventDefault()
    event.stopPropagation()
    this.input.handlePaste(event)
    this.refresh()
  }
}
