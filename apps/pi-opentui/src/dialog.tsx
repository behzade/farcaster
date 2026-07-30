import type { TextareaRenderable } from "@opentui/core"
import { useKeyboard } from "@opentui/solid"
import { For } from "solid-js"
import { SearchMenu } from "./search-menu.tsx"
import type {
  AppDialog,
  CommandInfo,
} from "./services/app-state.ts"

export function ExtensionDialog(props: {
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
  let secret = ""
  let secretCursor = 0

  const submitInput = () => {
    const value =
      props.dialog.kind === "secret"
        ? secret.trim()
        : input?.plainText.trim()
    props.resolve(value && value.length > 0 ? value : undefined)
  }

  const withSecretBuffer = (
    edit: (renderable: TextareaRenderable) => void,
  ) => {
    if (input === undefined) return
    input.editBuffer.setText(secret)
    input.cursorOffset = secretCursor
    edit(input)
    secret = input.plainText
    secretCursor = input.cursorOffset
    input.editBuffer.setText("•".repeat(secret.length))
    input.cursorOffset = secretCursor
  }

  const handleSecretKey = (
    event: Parameters<TextareaRenderable["handleKeyPress"]>[0],
  ) => {
    event.preventDefault()
    event.stopPropagation()
    withSecretBuffer((renderable) => renderable.handleKeyPress(event))
  }

  const handleSecretPaste = (
    event: Parameters<TextareaRenderable["handlePaste"]>[0],
  ) => {
    event.preventDefault()
    event.stopPropagation()
    withSecretBuffer((renderable) => renderable.handlePaste(event))
  }

  useKeyboard((event) => {
    if (event.name === "escape") {
      event.preventDefault()
      event.stopPropagation()
      props.resolve(undefined)
    }
  })

  const acceptsInput =
    props.dialog.kind === "input" ||
    props.dialog.kind === "secret"
  const inputColor =
    props.dialog.kind === "secret" ? "#928374" : "#ebdbb2"

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

      <For each={props.dialog.kind === "select" ? [props.dialog] : []}>
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

      <For each={acceptsInput ? [props.dialog] : []}>
        {() => (
          <textarea
            ref={(renderable) => {
              input = renderable
            }}
            focused
            height={2}
            placeholder={props.dialog.placeholder ?? "Type a response"}
            placeholderColor="#928374"
            textColor={inputColor}
            focusedTextColor={inputColor}
            backgroundColor="#1d2021"
            focusedBackgroundColor="#1d2021"
            cursorColor="#fabd2f"
            wrapMode="word"
            keyBindings={[
              { name: "return", action: "submit" },
              { name: "kpenter", action: "submit" },
              { name: "return", shift: true, action: "newline" },
            ]}
            {...(props.dialog.kind === "secret"
              ? {
                  onKeyDown: handleSecretKey,
                  onPaste: handleSecretPaste,
                }
              : {})}
            onSubmit={submitInput}
          />
        )}
      </For>

      <text fg="#928374">
        {props.dialog.kind === "select"
          ? "↑/↓ choose · enter confirm · esc cancel"
          : props.dialog.kind === "secret"
            ? "input hidden · enter confirm · esc cancel"
            : "enter confirm · shift+enter newline · esc cancel"}
      </text>
    </box>
  )
}

export function AuthNotice(props: {
  readonly message: string
}) {
  return (
    <box
      position="absolute"
      left="10%"
      top="20%"
      width="80%"
      minHeight={7}
      zIndex={15}
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
      <text fg="#fabd2f">
        <strong>Login</strong>
      </text>
      <text fg="#ebdbb2" wrapMode="word">
        {props.message}
      </text>
      <text fg="#928374">esc cancel</text>
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
