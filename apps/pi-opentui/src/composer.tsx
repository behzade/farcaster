import type {
  TextareaRenderable,
  TextRenderable,
} from "@opentui/core"
import { For, createSignal } from "solid-js"
import { CommandMenu } from "./dialog.tsx"
import type {
  AppCommand,
  AppSnapshot,
  CommandInfo,
} from "./services/app-state.ts"
import {
  exactSlashCommand,
  slashCommandMatches,
} from "./services/commands.ts"
import {
  applyFileMentionCompletion,
  fileMentionMatches,
  type FileCompletion,
} from "./services/file-completion.ts"
import type { ProjectPath } from "./services/project-paths.ts"

export interface ComposerProps {
  readonly snapshot: () => AppSnapshot
  readonly projectPaths: () => ReadonlyArray<ProjectPath>
  readonly dispatch: (command: AppCommand) => void
}

export function Composer(props: ComposerProps) {
  let input: TextareaRenderable | undefined
  let hint: TextRenderable | undefined
  let suggestionIndex = 0
  let cachedFileText = ""
  let cachedFileCursor = -1
  let cachedProjectPaths: ReadonlyArray<ProjectPath> | undefined
  let cachedFileSuggestions: ReadonlyArray<FileCompletion> = []
  const [paletteOpen, setPaletteOpen] = createSignal(false)

  const canComplete = (): boolean =>
    props.snapshot().dialog === undefined &&
    !paletteOpen() &&
    props.snapshot().phase !== "running" &&
    props.snapshot().phase !== "stopping"

  const commandSuggestions = (): ReadonlyArray<CommandInfo> =>
    slashCommandMatches(
      props.snapshot().commands,
      input?.plainText ?? "",
    )

  const fileSuggestions = (): ReadonlyArray<FileCompletion> => {
    const text = input?.plainText ?? ""
    const cursor = input?.cursorOffset ?? text.length
    const paths = props.projectPaths()
    if (
      text !== cachedFileText ||
      cursor !== cachedFileCursor ||
      paths !== cachedProjectPaths
    ) {
      cachedFileText = text
      cachedFileCursor = cursor
      cachedProjectPaths = paths
      cachedFileSuggestions = fileMentionMatches(
        paths,
        text,
        cursor,
      )
    }
    return cachedFileSuggestions
  }

  const activeCount = (): number => {
    if (!canComplete()) return 0
    const files = fileSuggestions()
    return files.length > 0 ? files.length : commandSuggestions().length
  }

  const selectedIndex = (length: number): number =>
    Math.min(suggestionIndex, length - 1)

  const updateHint = () => {
    if (hint === undefined) return
    const files = fileSuggestions()
    const file = files[selectedIndex(files.length)]
    if (canComplete() && file !== undefined) {
      hint.content = `${file.path}${file.isDirectory ? " · folder" : ""} · ↑/↓ choose · tab or enter complete`
      return
    }

    const commands = commandSuggestions()
    const command = commands[selectedIndex(commands.length)]
    hint.content =
      canComplete() && command !== undefined
        ? `/${command.name}${command.description.length > 0 ? ` — ${command.description}` : ""} · ↑/↓ choose · tab or enter complete`
        : "enter send · shift+enter newline · / commands · @ files"
  }

  const setDraft = (text: string, cursorOffset = text.length) => {
    input?.editBuffer.setText(text)
    if (input !== undefined) input.cursorOffset = cursorOffset
    input?.focus()
    suggestionIndex = 0
    updateHint()
  }

  const completeCommand = (command: CommandInfo) => {
    setDraft(`/${command.name} `)
  }

  const completeFile = (completion: FileCompletion) => {
    if (input === undefined) return
    const result = applyFileMentionCompletion(
      input.plainText,
      input.cursorOffset,
      completion,
    )
    if (result !== undefined) {
      setDraft(result.text, result.cursorOffset)
    }
  }

  const completeSelected = (): boolean => {
    if (!canComplete()) return false
    const files = fileSuggestions()
    const file = files[selectedIndex(files.length)]
    if (file !== undefined) {
      completeFile(file)
      return true
    }
    const commands = commandSuggestions()
    const command = commands[selectedIndex(commands.length)]
    if (command !== undefined) {
      completeCommand(command)
      return true
    }
    return false
  }

  const closePalette = (name: string | undefined) => {
    setPaletteOpen(false)
    if (name !== undefined && name.length > 0) {
      setDraft(`/${name} `)
    } else {
      input?.focus()
      updateHint()
    }
  }

  const submit = () => {
    const prompt = input?.plainText.trim() ?? ""
    if (prompt === "/" && props.snapshot().phase === "ready") {
      setPaletteOpen(true)
      return
    }
    if (fileSuggestions().length > 0 && completeSelected()) return
    if (
      prompt.startsWith("/") &&
      exactSlashCommand(props.snapshot().commands, prompt) === undefined &&
      completeSelected()
    ) {
      return
    }
    if (
      prompt.length === 0 ||
      props.snapshot().phase === "running" ||
      props.snapshot().phase === "stopping"
    ) {
      return
    }

    props.dispatch({ _tag: "Prompt", text: prompt })
    setDraft("")
  }

  const handleKey = (
    event: Parameters<TextareaRenderable["handleKeyPress"]>[0],
  ) => {
    const count = activeCount()
    if (
      count > 0 &&
      (event.name === "tab" ||
        event.name === "up" ||
        event.name === "down")
    ) {
      event.preventDefault()
      if (event.name === "tab") {
        completeSelected()
      } else {
        const offset = event.name === "up" ? -1 : 1
        suggestionIndex =
          (suggestionIndex + offset + count) % count
        updateHint()
      }
      return
    }
    if (
      ((event.name === "c" || event.name === "q") && event.ctrl) ||
      (event.name === "escape" &&
        (props.snapshot().phase === "running" ||
          props.snapshot().phase === "stopping"))
    ) {
      return
    }
    if (input === undefined) return
    event.preventDefault()
    input.handleKeyPress(event)
    suggestionIndex = 0
    updateHint()
  }

  const handlePaste = (
    event: Parameters<TextareaRenderable["handlePaste"]>[0],
  ) => {
    if (input === undefined) return
    event.preventDefault()
    input.handlePaste(event)
    suggestionIndex = 0
    updateHint()
  }

  return (
    <>
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
            props.snapshot().dialog === undefined && !paletteOpen()
          }
          height={2}
          placeholder={
            props.snapshot().phase === "running"
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
          onKeyDown={handleKey}
          onPaste={handlePaste}
          onSubmit={submit}
        />
        <box height={1} flexDirection="row" justifyContent="space-between">
          <text
            ref={(renderable) => {
              hint = renderable
            }}
            fg="#928374"
          >
            enter send · shift+enter newline · / commands · @ files
          </text>
          <text fg="#928374">ctrl+c quit</text>
        </box>
      </box>

      <For each={paletteOpen() ? [true] : []}>
        {() => (
          <CommandMenu
            commands={props.snapshot().commands}
            resolve={closePalette}
          />
        )}
      </For>
    </>
  )
}
