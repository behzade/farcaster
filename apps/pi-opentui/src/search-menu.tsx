import type {
  InputRenderable,
  SelectRenderable,
  TextRenderable,
} from "@opentui/core"

const resultLimit = 8

export interface SearchMenuProps {
  readonly title: string
  readonly message?: string
  readonly options: ReadonlyArray<string>
  readonly initialQuery?: string
  readonly resolve: (value: string | undefined) => void
}

export const filterOptions = (
  options: ReadonlyArray<string>,
  query: string,
): ReadonlyArray<string> => {
  const needle = query.trim().toLocaleLowerCase()
  if (needle.length === 0) return options
  return options.filter((option) =>
    option.toLocaleLowerCase().includes(needle)
  )
}

export const nearbyOptions = (
  options: ReadonlyArray<string>,
  selectedIndex: number,
  limit = resultLimit,
): ReadonlyArray<{ readonly index: number; readonly option: string }> => {
  if (options.length === 0 || limit <= 0) return []

  const safeIndex = Math.max(0, Math.min(selectedIndex, options.length - 1))
  const size = Math.min(limit, options.length)
  const start = Math.max(
    0,
    Math.min(safeIndex - Math.floor(size / 2), options.length - size),
  )

  return options
    .slice(start, start + size)
    .map((option, offset) => ({ index: start + offset, option }))
}

export function SearchMenu(props: SearchMenuProps) {
  let input: InputRenderable | undefined
  let select: SelectRenderable | undefined
  let empty: TextRenderable | undefined
  let resolved = false

  const finish = (value: string | undefined) => {
    if (resolved) return
    resolved = true
    props.resolve(value)
  }
  const currentOptions = (): ReadonlyArray<string> =>
    filterOptions(props.options, input?.value ?? props.initialQuery ?? "")
  const selectOptions = (options: ReadonlyArray<string>) =>
    options.map((option) => ({
      name: option,
      description: "",
      value: option,
    }))
  const refresh = () => {
    const options = currentOptions()
    if (select !== undefined) {
      select.options = selectOptions(options)
      select.setSelectedIndex(0)
    }
    if (empty !== undefined) {
      empty.content = options.length === 0 ? "No matches" : ""
    }
  }

  const handleKey = (event: Parameters<InputRenderable["handleKeyPress"]>[0]) => {
    if (event.name === "escape") {
      event.preventDefault()
      event.stopPropagation()
      finish(undefined)
      return
    }
    if (event.name === "up" || event.name === "down") {
      event.preventDefault()
      event.stopPropagation()
      if (event.name === "up") select?.moveUp()
      else select?.moveDown()
      return
    }
    if (
      event.name === "return" ||
      event.name === "kpenter"
    ) {
      event.preventDefault()
      event.stopPropagation()
      const value = select?.getSelectedOption()?.value
      finish(typeof value === "string" ? value : undefined)
      return
    }
    if (input === undefined) return
    event.preventDefault()
    event.stopPropagation()
    input.handleKeyPress(event)
    refresh()
  }
  const handlePaste = (
    event: Parameters<InputRenderable["handlePaste"]>[0],
  ) => {
    if (input === undefined) return
    event.preventDefault()
    event.stopPropagation()
    input.handlePaste(event)
    refresh()
  }

  const initialOptions = filterOptions(
    props.options,
    props.initialQuery ?? "",
  )

  return (
    <box
      position="absolute"
      left="10%"
      top="15%"
      width="80%"
      minHeight={9}
      zIndex={30}
      flexDirection="column"
      border
      borderColor="#fabd2f"
      backgroundColor="#282828"
      paddingLeft={2}
      paddingRight={2}
      paddingTop={1}
      paddingBottom={1}
      gap={1}
    >
      <text fg="#fabd2f" wrapMode="word">
        <strong>{props.title}</strong>
      </text>
      <text fg="#ebdbb2" wrapMode="word">
        {props.message ?? ""}
      </text>
      <input
        ref={(renderable) => {
          input = renderable
        }}
        focused
        value={props.initialQuery ?? ""}
        placeholder="Type to search"
        placeholderColor="#928374"
        textColor="#ebdbb2"
        focusedTextColor="#ebdbb2"
        backgroundColor="#1d2021"
        focusedBackgroundColor="#1d2021"
        cursorColor="#fabd2f"
        onKeyDown={handleKey}
        onPaste={handlePaste}
      />
      <select
        ref={(renderable) => {
          select = renderable
        }}
        height={resultLimit}
        options={selectOptions(initialOptions)}
        showDescription={false}
        wrapSelection
        backgroundColor="#282828"
        textColor="#ebdbb2"
        focusedBackgroundColor="#282828"
        focusedTextColor="#ebdbb2"
        selectedBackgroundColor="#504945"
        selectedTextColor="#fabd2f"
        onSelect={(_index, option) =>
          finish(
            typeof option?.value === "string"
              ? option.value
              : undefined,
          )
        }
      />
      <text
        ref={(renderable) => {
          empty = renderable
        }}
        fg="#928374"
      >
        {initialOptions.length === 0 ? "No matches" : ""}
      </text>
      <text fg="#928374">
        ↑/↓ choose · enter confirm · esc cancel
      </text>
    </box>
  )
}
