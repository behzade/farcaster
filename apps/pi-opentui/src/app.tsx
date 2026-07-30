import { useKeyboard } from "@opentui/solid"
import { For, createSignal, onCleanup } from "solid-js"
import { Composer } from "./composer.tsx"
import {
  AuthNotice,
  ExtensionDialog,
} from "./dialog.tsx"
import type {
  AppCommand,
  AppDialog,
  AppSnapshot,
} from "./services/app-state.ts"
import type { ProjectPath } from "./services/project-paths.ts"
import type {
  TranscriptRow,
  TranscriptRowKind,
} from "./services/transcript.ts"

export interface AppBridge {
  readonly initial: AppSnapshot
  readonly projectPaths: () => ReadonlyArray<ProjectPath>
  readonly subscribe: (listener: (snapshot: AppSnapshot) => void) => () => void
  readonly dispatch: (command: AppCommand) => void
  readonly quit: () => void
}

export { CommandMenu } from "./dialog.tsx"

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

export function App(props: AppProps) {
  const [snapshot, setSnapshot] = createSignal(props.bridge.initial)
  const dialogs = (): ReadonlyArray<AppDialog> => {
    const dialog = snapshot().dialog
    return dialog === undefined ? [] : [dialog]
  }
  const authNotices = (): ReadonlyArray<string> => {
    const notice = snapshot().authNotice
    return notice === undefined ? [] : [notice]
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
  onCleanup(props.bridge.subscribe(setSnapshot))

  useKeyboard((event) => {
    if (
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

      <Composer
        snapshot={snapshot}
        projectPaths={props.bridge.projectPaths}
        dispatch={props.bridge.dispatch}
      />

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

      <For each={authNotices()}>
        {(message) => <AuthNotice message={message} />}
      </For>
    </box>
  )
}
