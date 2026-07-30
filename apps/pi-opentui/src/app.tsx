import type {
  TextareaRenderable,
  TextRenderable,
} from "@opentui/core"
import { useKeyboard } from "@opentui/solid"
import { For, createSignal, onCleanup } from "solid-js"
import type {
  AppCommand,
  AppDialog,
  AppSnapshot,
  CommandInfo,
} from "./services/app-state.ts"
import {
  exactSlashCommand,
  slashCommandMatches,
} from "./services/commands.ts"
import { SearchMenu } from "./search-menu.tsx"
import type {
  TranscriptRow,
  TranscriptRowKind,
} from "./services/transcript.ts"

export interface AppBridge {
  readonly initial: AppSnapshot
  readonly subscribe: (listener: (snapshot: AppSnapshot) => void) => () => void
  readonly dispatch: (command: AppCommand) => void
  readonly quit: () => void
}

export interface AppProps {
  readonly bridge: AppBridge
}

const rowColor = (kind: TranscriptRowKind, isError: boolean): string => {
  if (isError || kind === "error") return "#fb4934"
  switch (kind) {
    case "user":
      return "#83a598"
    case "assistant":
      return "#b8bb26"
    case "tool":
      return "#d3869b"
    case "notice":
      return "#fabd2f"
  }
}

function TranscriptItem(props: { readonly row: TranscriptRow }) {
  return (
    <box flexDirection="column" paddingLeft={1} paddingRight={1}>
      <text fg={rowColor(props.row.kind, props.row.isError)}>
        <strong>{props.row.title}</strong>
        {props.row.pending ? " …" : ""}
      </text>
      <text
        fg={props.row.kind === "tool" ? "#a89984" : "#ebdbb2"}
        wrapMode="word"
      >
        {props.row.content}
      </text>
    </box>
  )
}

function ExtensionDialog(props: {
  readonly dialog: AppDialog
  readonly resolve: (value: string | undefined) => void
}) {
  if (props.dialog.kind === "search") {
    return (
      <SearchMenu
        title={props.dialog.title}
        options={props.dialog.options}
        resolve={props.resolve}
        {...(props.dialog.message === undefined
          ? {}
          : { message: props.dialog.message })}
        {...(props.dialog.initialQuery === undefined
          ? {}
          : { initialQuery: props.dialog.initialQuery })}
      />
    )
  }

  let input: TextareaRenderable | undefined

  const submitInput = () => {
    const value = input?.plainText.trim()
    props.resolve(value && value.length > 0 ? value : undefined)
  }

  useKeyboard((event) => {
    if (event.name === "escape") {
      event.preventDefault()
      event.stopPropagation()
      props.resolve(undefined)
      return
    }
  })

  return (
    <box
      position="absolute"
      left="10%"
      top="20%"
      width="80%"
      minHeight={8}
      zIndex={20}
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
        <strong>{props.dialog.title}</strong>
      </text>
      <text fg="#ebdbb2" wrapMode="word">
        {props.dialog.message ?? ""}
      </text>

      <For
        each={
          props.dialog.kind === "select"
            ? [props.dialog]
            : []
        }
      >
        {(dialog) => (
          <select
            focused
            height={Math.max(1, Math.min(8, dialog.options.length))}
            options={dialog.options.map((option) => ({
              name: option,
              description: "",
              value: option,
            }))}
            showDescription={false}
            backgroundColor="#282828"
            textColor="#ebdbb2"
            focusedBackgroundColor="#282828"
            focusedTextColor="#ebdbb2"
            selectedBackgroundColor="#504945"
            selectedTextColor="#fabd2f"
            onSelect={(_index, option) =>
              props.resolve(
                typeof option?.value === "string"
                  ? option.value
                  : undefined,
              )
            }
          />
        )}
      </For>

      <For
        each={props.dialog.kind === "input" ? [props.dialog] : []}
      >
        {() => (
          <textarea
            ref={(renderable) => {
              input = renderable
            }}
            focused
            height={2}
            placeholder={props.dialog.placeholder ?? "Type a response"}
            placeholderColor="#928374"
            textColor="#ebdbb2"
            focusedTextColor="#ebdbb2"
            backgroundColor="#1d2021"
            focusedBackgroundColor="#1d2021"
            cursorColor="#fabd2f"
            wrapMode="word"
            keyBindings={[
              { name: "return", action: "submit" },
              { name: "kpenter", action: "submit" },
              { name: "return", shift: true, action: "newline" },
            ]}
            onSubmit={submitInput}
          />
        )}
      </For>

      <text fg="#928374">
        {props.dialog.kind === "select"
          ? "↑/↓ choose · enter confirm · esc cancel"
          : "enter confirm · shift+enter newline · esc cancel"}
      </text>
    </box>
  )
}

export function CommandMenu(props: {
  readonly commands: ReadonlyArray<CommandInfo>
  readonly resolve: (name: string | undefined) => void
}) {
  const options = props.commands.map(
    (command) =>
      `/${command.name}${command.description.length > 0 ? ` — ${command.description}` : ""}`,
  )

  return (
    <SearchMenu
      title="Commands"
      message="Choose a command to run."
      options={options}
      resolve={(selected) =>
        props.resolve(selected?.slice(1).split(/\s/, 1)[0])
      }
    />
  )
}

export function App(props: AppProps) {
  let input: TextareaRenderable | undefined
  let commandHint: TextRenderable | undefined
  let commandIndex = 0
  const [snapshot, setSnapshot] = createSignal(props.bridge.initial)
  const [paletteOpen, setPaletteOpen] = createSignal(false)
  const dialogs = (): ReadonlyArray<AppDialog> => {
    const dialog = snapshot().dialog
    return dialog === undefined ? [] : [dialog]
  }
  const modelText = (): string => {
    const model = snapshot().model
    if (model === undefined) return "no model"
    const thinking = model.reasoning
      ? ` · thinking ${snapshot().thinkingLevel}`
      : ""
    return `${model.provider}/${model.id}${thinking}`
  }
  const usageText = (): string => {
    const stats = snapshot().sessionStats
    const context =
      stats.contextUsage?.percent == null
        ? ""
        : ` · ctx ${Math.round(stats.contextUsage.percent)}%`
    return `${snapshot().phase} · ${stats.tokens.total.toLocaleString()} tokens${context} · $${stats.cost.toFixed(4)}`
  }
  const commandSuggestions = (): ReadonlyArray<CommandInfo> =>
    slashCommandMatches(
      snapshot().commands,
      input?.plainText ?? "",
    )
  const commandSuggestionsVisible = (): boolean =>
    snapshot().dialog === undefined &&
    !paletteOpen() &&
    snapshot().phase !== "running" &&
    snapshot().phase !== "stopping" &&
    commandSuggestions().length > 0
  const updateCommandHint = () => {
    if (commandHint === undefined) return
    const suggestions = commandSuggestions()
    const suggestion =
      suggestions[
        Math.min(commandIndex, suggestions.length - 1)
      ]
    commandHint.content =
      commandSuggestionsVisible() && suggestion !== undefined
        ? `/${suggestion.name}${suggestion.description.length > 0 ? ` — ${suggestion.description}` : ""} · ↑/↓ choose · tab or enter complete`
        : "enter send · shift+enter newline · / enter commands"
  }

  onCleanup(props.bridge.subscribe(setSnapshot))

  const closePalette = (name: string | undefined) => {
    setPaletteOpen(false)
    if (name === undefined || name.length === 0) return
    const command = `/${name} `
    input?.editBuffer.setText(command)
    input?.focus()
    commandIndex = 0
    updateCommandHint()
  }

  const completeCommand = (command: CommandInfo) => {
    const value = `/${command.name} `
    input?.editBuffer.setText(value)
    input?.focus()
    commandIndex = 0
    updateCommandHint()
  }

  const submit = () => {
    const prompt = input?.plainText.trim() ?? ""
    if (prompt === "/" && snapshot().phase === "ready") {
      setPaletteOpen(true)
      return
    }
    if (
      prompt.startsWith("/") &&
      exactSlashCommand(snapshot().commands, prompt) === undefined
    ) {
      const suggestion =
        commandSuggestions()[
          Math.min(commandIndex, commandSuggestions().length - 1)
        ]
      if (suggestion !== undefined) {
        completeCommand(suggestion)
        return
      }
    }
    if (
      prompt.length === 0 ||
      snapshot().phase === "running" ||
      snapshot().phase === "stopping"
    ) {
      return
    }

    props.bridge.dispatch({ _tag: "Prompt", text: prompt })
    input?.editBuffer.setText("")
    commandIndex = 0
    updateCommandHint()
  }

  const handleComposerKey = (
    event: Parameters<TextareaRenderable["handleKeyPress"]>[0],
  ) => {
    const suggestions = commandSuggestions()
    if (
      commandSuggestionsVisible() &&
      (event.name === "tab" ||
        event.name === "up" ||
        event.name === "down")
    ) {
      event.preventDefault()
      if (event.name === "tab") {
        const suggestion =
          suggestions[
            Math.min(commandIndex, suggestions.length - 1)
          ]
        if (suggestion !== undefined) completeCommand(suggestion)
      } else {
        const offset = event.name === "up" ? -1 : 1
        commandIndex =
          (commandIndex + offset + suggestions.length) %
          suggestions.length
        updateCommandHint()
      }
      return
    }
    if (
      (event.name === "c" || event.name === "q") &&
      event.ctrl
    ) {
      event.preventDefault()
      props.bridge.quit()
      return
    }
    if (
      event.name === "escape" &&
      (snapshot().phase === "running" ||
        snapshot().phase === "stopping")
    ) {
      event.preventDefault()
      props.bridge.dispatch({ _tag: "Abort" })
      return
    }
    if (input === undefined) return
    event.preventDefault()
    input.handleKeyPress(event)
    commandIndex = 0
    updateCommandHint()
  }

  const handleComposerPaste = (
    event: Parameters<TextareaRenderable["handlePaste"]>[0],
  ) => {
    if (input === undefined) return
    event.preventDefault()
    input.handlePaste(event)
    commandIndex = 0
    updateCommandHint()
  }

  useKeyboard((event) => {
    const suggestions = commandSuggestions()
    if (
      commandSuggestionsVisible() &&
      (event.name === "tab" ||
        event.name === "up" ||
        event.name === "down")
    ) {
      event.preventDefault()
      event.stopPropagation()
      if (event.name === "tab") {
        const suggestion =
          suggestions[
            Math.min(commandIndex, suggestions.length - 1)
          ]
        if (suggestion !== undefined) completeCommand(suggestion)
      } else {
        const offset = event.name === "up" ? -1 : 1
        commandIndex =
          (commandIndex + offset + suggestions.length) %
          suggestions.length
        updateCommandHint()
      }
    } else if (
      (event.name === "c" || event.name === "q") &&
      event.ctrl
    ) {
      event.preventDefault()
      event.stopPropagation()
      props.bridge.quit()
    } else if (
      event.name === "escape" &&
      snapshot().dialog === undefined &&
      (snapshot().phase === "running" ||
        snapshot().phase === "stopping")
    ) {
      event.preventDefault()
      event.stopPropagation()
      props.bridge.dispatch({ _tag: "Abort" })
    }
  })

  return (
    <box
      width="100%"
      height="100%"
      flexDirection="column"
      paddingLeft={1}
      paddingRight={1}
      backgroundColor="#1d2021"
    >
      <box
        height={1}
        flexDirection="row"
        justifyContent="space-between"
      >
        <box flexDirection="row">
          <text fg="#fabd2f">
            <strong>pi-next</strong>
          </text>
          <text fg="#928374">
            {" "}
            · {snapshot().activeTools.length} tools ·{" "}
            {snapshot().extensionPaths.length} extensions
          </text>
        </box>
        <text fg="#a89984">{usageText()}</text>
      </box>

      <text fg="#928374" wrapMode="none">
        {snapshot().cwd} · {modelText()}
        {Object.values(snapshot().statuses).length > 0
          ? ` · ${Object.values(snapshot().statuses).join(" · ")}`
          : ""}
      </text>

      <scrollbox
        flexGrow={1}
        stickyScroll
        stickyStart="bottom"
        viewportCulling
        scrollY
      >
        <box flexDirection="column" gap={1}>
          <text fg="#928374">
            {snapshot().transcript.rows.length === 0
              ? "Type a prompt to start."
              : ""}
          </text>
          <For each={snapshot().transcript.rows}>
            {(row) => <TranscriptItem row={row} />}
          </For>
        </box>
      </scrollbox>

      <box
        height={4}
        flexDirection="column"
        border={["top"]}
        borderColor="#504945"
        paddingLeft={1}
        paddingRight={1}
      >
        <textarea
          ref={(renderable) => {
            input = renderable
          }}
          focused={
            snapshot().dialog === undefined && !paletteOpen()
          }
          height={2}
          placeholder={
            snapshot().phase === "running"
              ? "Pi is working…"
              : "Ask Pi"
          }
          placeholderColor="#928374"
          textColor="#ebdbb2"
          focusedTextColor="#ebdbb2"
          backgroundColor="#1d2021"
          focusedBackgroundColor="#1d2021"
          cursorColor="#fabd2f"
          wrapMode="word"
          scrollMargin={0}
          keyBindings={[
            { name: "return", action: "submit" },
            { name: "kpenter", action: "submit" },
            { name: "return", shift: true, action: "newline" },
            { name: "kpenter", shift: true, action: "newline" },
          ]}
          onKeyDown={handleComposerKey}
          onPaste={handleComposerPaste}
          onSubmit={submit}
        />
        <box height={1} flexDirection="row" justifyContent="space-between">
          <text
            ref={(renderable) => {
              commandHint = renderable
            }}
            fg="#928374"
          >
            enter send · shift+enter newline · / enter commands
          </text>
          <text fg="#928374">ctrl+c quit</text>
        </box>
      </box>

      <For
        each={dialogs()}
      >
        {(dialog) => (
          <ExtensionDialog
            dialog={dialog}
            resolve={(value) =>
              props.bridge.dispatch({
                _tag: "ResolveDialog",
                id: dialog.id,
                value,
              })
            }
          />
        )}
      </For>

      <For each={paletteOpen() ? [true] : []}>
        {() => (
          <CommandMenu
            commands={snapshot().commands}
            resolve={closePalette}
          />
        )}
      </For>
    </box>
  )
}
