import {
  createTestRenderer,
  type TestRendererSetup,
} from "@opentui/core/testing"
import { expect, test } from "bun:test"
import { Effect } from "effect"
import { KeybindingsManager } from "@earendil-works/pi-coding-agent"
import {
  AppView,
  mountApp,
} from "../src/opentui/app-view.ts"
import { createCommandMenu } from "../src/opentui/dialog-view.ts"
import type {
  AppCommand,
  AppSnapshot,
} from "../src/services/app-state.ts"
import { emptyLiveUsage } from "../src/services/live-usage.ts"
import {
  makeKeybindings,
  type KeybindingsShape,
} from "../src/services/keybindings.ts"
import type { AppClient } from "../src/ui/app-client.ts"
import type {
  PasteInsertion,
  PasteRequest,
} from "../src/services/paste-model.ts"

const baseSnapshot: AppSnapshot = {
  cwd: "/work/pi",
  hideThinkingBlock: false,
  phase: "ready",
  activeTools: ["read", "sandbox"],
  model: {
    provider: "openai",
    id: "gpt-5",
    name: "GPT-5",
    reasoning: true,
  },
  thinkingLevel: "high",
  sessionStats: {
    sessionFile: undefined,
    sessionId: "session-1",
    userMessages: 1,
    assistantMessages: 1,
    toolCalls: 0,
    toolResults: 0,
    totalMessages: 2,
    tokens: {
      input: 1000,
      output: 200,
      cacheRead: 300,
      cacheWrite: 0,
      total: 1500,
    },
    cost: 0.0123,
    contextUsage: {
      tokens: 32000,
      contextWindow: 128000,
      percent: 25,
    },
  },
  liveUsage: emptyLiveUsage,
  extensionPaths: ["/agent/extensions/sandbox"],
  extensionErrors: [],
  eventCount: 3,
  lastEvent: "agent_settled",
  error: undefined,
  transcript: {
    rows: [
      {
        id: "row-1",
        kind: "user",
        title: "you",
        content: "hello from user",
        pending: false,
        isError: false,
      },
    ],
    activeAssistantId: undefined,
    nextRowId: 2,
  },
  dialog: undefined,
  authNotice: undefined,
  statuses: {},
  promptQueue: { steering: [], followUp: [] },
  draftRestore: undefined,
  commands: [
    {
      name: "session",
      description: "Show session info and stats",
      source: "builtin",
    },
  ],
}

type PasteResolver = (
  request: PasteRequest,
  accept: (insertion: PasteInsertion | undefined) => void,
) => void

const makeClient = (
  initial: AppSnapshot = baseSnapshot,
  resolvePaste: PasteResolver = () => undefined,
) => {
  const commands: Array<AppCommand> = []
  const listeners = new Set<(snapshot: AppSnapshot) => void>()
  let quitCalls = 0
  const client: AppClient = {
    initial,
    projectPaths: () => [
      { path: "src/", isDirectory: true },
      { path: "src/opentui/app-view.ts", isDirectory: false },
    ],
    subscribe: (listener) => {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
    dispatch: (command) => {
      commands.push(command)
    },
    resolvePaste,
    quit: () => {
      quitCalls += 1
    },
  }
  return {
    client,
    commands,
    emit: (snapshot: AppSnapshot) => {
      for (const listener of listeners) listener(snapshot)
    },
    listenerCount: () => listeners.size,
    quitCalls: () => quitCalls,
  }
}

interface MountedApp {
  readonly setup: TestRendererSetup
  readonly view: AppView
}

const acquireApp = (
  client: AppClient,
  width = 100,
  height = 30,
  keybindings: KeybindingsShape = makeKeybindings(
    new KeybindingsManager(),
  ),
): Effect.Effect<MountedApp, unknown> =>
  Effect.gen(function* () {
    const setup = yield* Effect.tryPromise(() =>
      createTestRenderer({
        width,
        height,
        exitOnCtrlC: false,
        kittyKeyboard: true,
      })
    )
    const view = yield* Effect.sync(() =>
      mountApp(setup.renderer, client, keybindings),
    )
    return { setup, view }
  })

const releaseApp = ({ setup, view }: MountedApp): Effect.Effect<void> =>
  Effect.sync(() => {
    view.destroy()
    setup.renderer.destroy()
  })

test("renders chat and sends input", () => {
  const state = makeClient()
  return Effect.runPromise(
    Effect.acquireUseRelease(
      acquireApp(state.client),
      ({ setup }) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() => setup.renderOnce())
          const frame = setup.captureCharFrame()

          expect(frame).toContain("pi-next")
          expect(frame).toContain("/work/pi")
          expect(frame).toContain("openai/gpt-5")
          expect(frame).toContain("thinking high")
          expect(frame).toContain("1,500 tokens")
          expect(frame).toContain("ctx 25%")
          expect(frame).toContain("$0.0123")
          expect(frame).toContain("hello from user")

          yield* Effect.tryPromise(() =>
            setup.mockInput.typeText("run tests")
          )
          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())

          expect(state.commands).toEqual([
            { _tag: "Prompt", text: "run tests", delivery: "steer" },
          ])
        }),
      releaseApp,
    ),
  )
})

test("uses Pi keybindings without treating ctrl+shift+c as ctrl+c", () => {
  const state = makeClient()
  return Effect.runPromise(
    Effect.acquireUseRelease(
      acquireApp(state.client),
      ({ setup }) =>
        Effect.gen(function* () {
          setup.mockInput.pressKey("c", { ctrl: true, shift: true })
          yield* Effect.tryPromise(() => setup.flush())
          expect(state.quitCalls()).toBe(0)

          yield* Effect.tryPromise(() => setup.mockInput.typeText("draft"))
          setup.mockInput.pressCtrlC()
          yield* Effect.tryPromise(() => setup.flush())
          expect(setup.captureCharFrame()).not.toContain("draft")
          expect(state.quitCalls()).toBe(0)

          setup.mockInput.pressCtrlC()
          yield* Effect.tryPromise(() => setup.flush())
          expect(state.quitCalls()).toBe(1)
        }),
      releaseApp,
    ),
  )
})

test("steers, queues follow-ups, and restores Pi queued messages", () => {
  const running = {
    ...baseSnapshot,
    phase: "running" as const,
    promptQueue: {
      steering: ["change the tests"],
      followUp: ["then write a summary"],
    },
  }
  const state = makeClient(running)
  return Effect.runPromise(
    Effect.acquireUseRelease(
      acquireApp(state.client),
      ({ setup }) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() => setup.renderOnce())
          expect(setup.captureCharFrame()).toContain(
            "steering: change the tests",
          )

          yield* Effect.tryPromise(() => setup.mockInput.typeText("use bun"))
          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(state.commands).toContainEqual({
            _tag: "Prompt",
            text: "use bun",
            delivery: "steer",
          })

          yield* Effect.tryPromise(() =>
            setup.mockInput.typeText("report timing"),
          )
          setup.mockInput.pressEnter({ meta: true })
          yield* Effect.tryPromise(() => setup.flush())
          expect(state.commands).toContainEqual({
            _tag: "Prompt",
            text: "report timing",
            delivery: "followUp",
          })

          setup.mockInput.pressArrow("up", { meta: true })
          yield* Effect.tryPromise(() => setup.flush())
          expect(state.commands).toContainEqual({ _tag: "Dequeue" })

          state.emit({
            ...running,
            promptQueue: { steering: [], followUp: [] },
            draftRestore: {
              id: 4,
              text: "change the tests\n\nthen write a summary",
            },
          })
          yield* Effect.tryPromise(() => setup.flush())
          expect(setup.captureCharFrame()).toContain("then write a summary")
          expect(state.commands).toContainEqual({
            _tag: "AcknowledgeDraftRestore",
            id: 4,
          })
          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(state.commands).toContainEqual({
            _tag: "Prompt",
            text: "change the tests\n\nthen write a summary",
            delivery: "steer",
          })
        }),
      releaseApp,
    ),
  )
})

test("honors Pi user overrides for the exit binding", () => {
  const state = makeClient()
  const keybindings = makeKeybindings(
    new KeybindingsManager({ "app.exit": "ctrl+q" }),
  )
  return Effect.runPromise(
    Effect.acquireUseRelease(
      acquireApp(state.client, 100, 30, keybindings),
      ({ setup }) =>
        Effect.gen(function* () {
          setup.mockInput.pressKey("d", { ctrl: true })
          yield* Effect.tryPromise(() => setup.flush())
          expect(state.quitCalls()).toBe(0)

          setup.mockInput.pressKey("q", { ctrl: true })
          yield* Effect.tryPromise(() => setup.flush())
          expect(state.quitCalls()).toBe(1)
        }),
      releaseApp,
    ),
  )
})

test("stores a large terminal paste and submits its file path", () => {
  const requests: Array<PasteRequest> = []
  const state = makeClient(baseSnapshot, (request, accept) => {
    requests.push(request)
    accept({ kind: "file", text: "/tmp/pi-paste-large.txt" })
  })

  return Effect.runPromise(
    Effect.acquireUseRelease(
      acquireApp(state.client),
      ({ setup }) =>
        Effect.gen(function* () {
          const content = "x".repeat(1_001)
          yield* Effect.tryPromise(() =>
            setup.mockInput.pasteBracketedText(content)
          )
          yield* Effect.tryPromise(() => setup.flush())

          expect(requests).toEqual([{ kind: "text", text: content }])
          expect(setup.captureCharFrame()).toContain(
            "/tmp/pi-paste-large.txt",
          )

          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(state.commands).toContainEqual({
            _tag: "Prompt",
            text: "/tmp/pi-paste-large.txt",
            delivery: "steer",
          })
        }),
      releaseApp,
    ),
  )
})

test("keeps a small terminal paste inline", () => {
  const requests: Array<PasteRequest> = []
  const state = makeClient(baseSnapshot, (request) => {
    requests.push(request)
  })

  return Effect.runPromise(
    Effect.acquireUseRelease(
      acquireApp(state.client),
      ({ setup }) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() =>
            setup.mockInput.pasteBracketedText("short paste"),
          )
          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())

          expect(requests).toEqual([])
          expect(state.commands).toContainEqual({
            _tag: "Prompt",
            text: "short paste",
            delivery: "steer",
          })
        }),
      releaseApp,
    ),
  )
})

test("pastes a clipboard image path at the cursor", () => {
  const requests: Array<PasteRequest> = []
  let finishPaste:
    | ((insertion: PasteInsertion | undefined) => void)
    | undefined
  const state = makeClient(baseSnapshot, (request, accept) => {
    requests.push(request)
    finishPaste = accept
  })

  return Effect.runPromise(
    Effect.acquireUseRelease(
      acquireApp(state.client),
      ({ setup }) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() => setup.mockInput.typeText("inspect"))
          setup.mockInput.pressKey("v", { ctrl: true })
          yield* Effect.tryPromise(() => setup.mockInput.typeText(" later"))
          yield* Effect.tryPromise(() => setup.flush())

          expect(requests).toEqual([{ kind: "clipboard" }])
          expect(setup.captureCharFrame()).toContain("[paste #1 ")
          expect(setup.captureCharFrame()).toContain(" loading]")
          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(state.commands).toEqual([])

          finishPaste?.({
            kind: "file",
            text: "/tmp/pi-clipboard-image.png",
          })
          yield* Effect.tryPromise(() => setup.flush())
          expect(setup.captureCharFrame()).toContain(
            "inspect /tmp/pi-clipboard-image.png later",
          )

          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(state.commands).toContainEqual({
            _tag: "Prompt",
            text: "inspect /tmp/pi-clipboard-image.png later",
            delivery: "steer",
          })
        }),
      releaseApp,
    ),
  )
})

test("keeps concurrent clipboard pastes in request order", () => {
  const completions: Array<
    (insertion: PasteInsertion | undefined) => void
  > = []
  const state = makeClient(baseSnapshot, (_request, accept) => {
    completions.push(accept)
  })

  return Effect.runPromise(
    Effect.acquireUseRelease(
      acquireApp(state.client),
      ({ setup }) =>
        Effect.gen(function* () {
          setup.mockInput.pressKey("v", { ctrl: true })
          setup.mockInput.pressKey("v", { ctrl: true })
          yield* Effect.tryPromise(() => setup.flush())
          expect(completions).toHaveLength(2)

          completions[1]?.({ kind: "file", text: "/tmp/second.png" })
          completions[0]?.({ kind: "file", text: "/tmp/first.png" })
          yield* Effect.tryPromise(() => setup.flush())

          expect(setup.captureCharFrame()).toContain(
            "/tmp/first.png /tmp/second.png",
          )
        }),
      releaseApp,
    ),
  )
})

test("resolves an extension selection dialog", () => {
  const state = makeClient({
    ...baseSnapshot,
    phase: "running",
    dialog: {
      id: 7,
      kind: "select",
      title: "Allow write?",
      message: "/tmp/output",
      options: ["Allow once", "Deny"],
      placeholder: undefined,
    },
  })

  return Effect.runPromise(
    Effect.acquireUseRelease(
      acquireApp(state.client, 80, 24),
      ({ setup }) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() => setup.renderOnce())
          yield* Effect.tryPromise(() => setup.flush())
          expect(setup.captureCharFrame()).toContain("Allow write?")

          setup.mockInput.pressArrow("down")
          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())

          expect(state.commands).toContainEqual({
            _tag: "ResolveDialog",
            id: 7,
            value: "Deny",
          })
        }),
      releaseApp,
    ),
  )
})

test("completes slash commands while typing", () => {
  const state = makeClient({
    ...baseSnapshot,
    commands: [
      ...baseSnapshot.commands,
      {
        name: "resume",
        description: "Resume a saved session",
        source: "builtin",
      },
    ],
  })

  return Effect.runPromise(
    Effect.acquireUseRelease(
      acquireApp(state.client),
      ({ setup }) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() => setup.mockInput.typeText("/res"))
          yield* Effect.tryPromise(() => setup.flush())
          expect(setup.captureCharFrame()).toContain(
            "/resume — Resume a saved session",
          )

          setup.mockInput.pressTab()
          yield* Effect.tryPromise(() => setup.flush())
          expect(setup.captureCharFrame()).toContain("/resume")

          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(state.commands).toContainEqual({
            _tag: "RunCommand",
            name: "resume",
            arguments: "",
            delivery: "steer",
          })
        }),
      releaseApp,
    ),
  )
})

test("completes file mentions while typing", () => {
  const state = makeClient()
  return Effect.runPromise(
    Effect.acquireUseRelease(
      acquireApp(state.client),
      ({ setup }) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() =>
            setup.mockInput.typeText("check @app")
          )
          yield* Effect.tryPromise(() => setup.flush())
          expect(setup.captureCharFrame()).toContain(
            "src/opentui/app-view.ts",
          )

          setup.mockInput.pressTab()
          yield* Effect.tryPromise(() => setup.flush())
          expect(setup.captureCharFrame()).toContain(
            "check @src/opentui/app-view.ts",
          )

          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(state.commands).toContainEqual({
            _tag: "Prompt",
            text: "check @src/opentui/app-view.ts",
            delivery: "steer",
          })
        }),
      releaseApp,
    ),
  )
})

test("hides login secrets while resolving them", () => {
  const state = makeClient({
    ...baseSnapshot,
    phase: "running",
    dialog: {
      id: 9,
      kind: "secret",
      title: "Enter API key",
      message: undefined,
      options: [],
      placeholder: "key",
    },
  })

  return Effect.runPromise(
    Effect.acquireUseRelease(
      acquireApp(state.client, 80, 24),
      ({ setup }) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() =>
            setup.mockInput.typeText("private-key")
          )
          yield* Effect.tryPromise(() => setup.flush())
          const frame = setup.captureCharFrame()
          expect(frame).toContain("input hidden")
          expect(frame).not.toContain("private-key")

          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(state.commands).toContainEqual({
            _tag: "ResolveDialog",
            id: 9,
            value: "private-key",
          })
        }),
      releaseApp,
    ),
  )
})

test("chooses a command from the slash menu", () => {
  let selected: string | undefined
  return Effect.runPromise(
    Effect.acquireUseRelease(
      Effect.gen(function* () {
        const setup = yield* Effect.tryPromise(() =>
          createTestRenderer({ width: 80, height: 24 })
        )
        const view = createCommandMenu(
          setup.renderer,
          baseSnapshot.commands,
          (name) => {
            selected = name
          },
        )
        setup.renderer.root.add(view.root)
        view.focus()
        return { setup, view }
      }),
      ({ setup }) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() => setup.renderOnce())
          expect(setup.captureCharFrame()).toContain("Commands")

          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(selected).toBe("session")
        }),
      ({ setup, view }) =>
        Effect.sync(() => {
          view.destroy()
          setup.renderer.destroy()
        }),
    ),
  )
})

test("updates a streaming row without duplicating it and unsubscribes", () => {
  const state = makeClient()
  return Effect.runPromise(
    Effect.acquireUseRelease(
      acquireApp(state.client),
      ({ setup, view }) =>
        Effect.gen(function* () {
          expect(state.listenerCount()).toBe(1)
          state.emit({
            ...baseSnapshot,
            transcript: {
              ...baseSnapshot.transcript,
              rows: [
                {
                  ...baseSnapshot.transcript.rows[0]!,
                  content: "streaming update",
                  pending: true,
                },
              ],
            },
          })
          yield* Effect.tryPromise(() => setup.flush())
          const frame = setup.captureCharFrame()
          expect(frame).toContain("streaming update")
          expect(frame.match(/streaming update/g)).toHaveLength(1)

          view.destroy()
          expect(state.listenerCount()).toBe(0)
        }),
      releaseApp,
    ),
  )
})

test("replaces dialogs and restores composer focus", () => {
  const state = makeClient()
  const firstDialog: AppSnapshot = {
    ...baseSnapshot,
    phase: "running",
    dialog: {
      id: 11,
      kind: "select",
      title: "First dialog",
      message: undefined,
      options: ["One"],
      placeholder: undefined,
    },
  }
  const secondDialog: AppSnapshot = {
    ...firstDialog,
    dialog: {
      id: 12,
      kind: "input",
      title: "Second dialog",
      message: undefined,
      options: [],
      placeholder: "value",
    },
  }

  return Effect.runPromise(
    Effect.acquireUseRelease(
      acquireApp(state.client),
      ({ setup }) =>
        Effect.gen(function* () {
          state.emit(firstDialog)
          yield* Effect.tryPromise(() => setup.flush())
          expect(setup.captureCharFrame()).toContain("First dialog")

          state.emit(secondDialog)
          yield* Effect.tryPromise(() => setup.flush())
          const replaced = setup.captureCharFrame()
          expect(replaced).toContain("Second dialog")
          expect(replaced).not.toContain("First dialog")

          setup.mockInput.pressEscape()
          yield* Effect.sleep("30 millis")
          yield* Effect.tryPromise(() => setup.flush())
          expect(state.commands).toContainEqual({
            _tag: "ResolveDialog",
            id: 12,
            value: undefined,
          })

          state.emit({
            ...baseSnapshot,
            dialog: undefined,
          })
          yield* Effect.tryPromise(() => setup.flush())
          yield* Effect.tryPromise(() => setup.mockInput.typeText("focused"))
          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(state.commands).toContainEqual({
            _tag: "Prompt",
            text: "focused",
            delivery: "steer",
          })
        }),
      releaseApp,
    ),
  )
})

test("closes the command palette and restores composer focus", () => {
  const state = makeClient()
  return Effect.runPromise(
    Effect.acquireUseRelease(
      acquireApp(state.client),
      ({ setup }) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() => setup.mockInput.typeText("/"))
          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(setup.captureCharFrame()).toContain("Commands")
          setup.mockInput.pressEscape()
          yield* Effect.sleep("30 millis")
          yield* Effect.tryPromise(() => setup.flush())
          expect(setup.captureCharFrame()).not.toContain("Commands")
          setup.mockInput.pressBackspace()
          yield* Effect.tryPromise(() => setup.mockInput.typeText("hello"))
          yield* Effect.tryPromise(() => setup.flush())
          expect(setup.captureCharFrame()).toContain("hello")
          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(state.commands).toContainEqual({
            _tag: "Prompt",
            text: "hello",
            delivery: "steer",
          })
        }),
      releaseApp,
    ),
  )
})
