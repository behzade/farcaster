import { useKeyboard } from "@opentui/solid"
import { For, createSignal, onCleanup } from "solid-js"
import type { AppCommand, AppSnapshot } from "./services/app-state.ts"

export interface AppBridge {
  readonly initial: AppSnapshot
  readonly subscribe: (listener: (snapshot: AppSnapshot) => void) => () => void
  readonly dispatch: (command: AppCommand) => void
  readonly quit: () => void
}

export interface AppProps {
  readonly bridge: AppBridge
}

const extensionName = (path: string): string => {
  const parts = path.split("/")
  const file = parts.at(-1) ?? path
  if (file !== "index.ts") return file

  const parent = parts.at(-2)
  return parent === "src"
    ? (parts.at(-3) ?? parent)
    : (parent ?? file)
}

export function App(props: AppProps) {
  const [snapshot, setSnapshot] = createSignal(props.bridge.initial)

  onCleanup(props.bridge.subscribe(setSnapshot))

  const quit = () => {
    props.bridge.quit()
  }

  useKeyboard((event) => {
    if (event.name === "q" || (event.name === "c" && event.ctrl)) {
      event.preventDefault()
      event.stopPropagation()
      quit()
    } else if (event.name === "escape" && snapshot().phase === "running") {
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
      paddingLeft={2}
      paddingRight={2}
      paddingTop={1}
      paddingBottom={1}
      gap={1}
      backgroundColor="#1d2021"
    >
      <box flexDirection="row" justifyContent="space-between">
        <text fg="#fabd2f">
          <strong>pi-next</strong>
        </text>
        <text fg="#a89984">{snapshot().phase}</text>
      </box>

      <box flexDirection="column">
        <text fg="#a89984">cwd</text>
        <text fg="#ebdbb2" wrapMode="none">
          {snapshot().cwd}
        </text>
      </box>

      <box flexDirection="column">
        <text fg="#a89984">
          Pi SDK · {snapshot().extensionPaths.length} extensions ·{" "}
          {snapshot().activeTools.length} active tools
        </text>
        <text fg="#b8bb26" wrapMode="none">
          {snapshot().activeTools.join("  ")}
        </text>
      </box>

      <box flexDirection="column">
        <text fg="#a89984">loaded extensions</text>
        <text fg="#ebdbb2" wrapMode="none">
          {snapshot().extensionPaths.map(extensionName).join("  ")}
        </text>
      </box>

      <For each={snapshot().extensionErrors}>
        {(fault) => (
          <text fg="#fb4934" wrapMode="none">
            {fault.path}: {fault.error}
          </text>
        )}
      </For>

      <text fg="#fb4934">{snapshot().error ?? ""}</text>

      <box flexGrow={1} />
      <text fg="#a89984">
        events {snapshot().eventCount} · last{" "}
        {snapshot().lastEvent ?? "none"}
      </text>
      <text fg="#928374">q quit · esc stop</text>
    </box>
  )
}
