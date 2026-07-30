import { testRender } from "@opentui/solid"
import { expect, test } from "bun:test"
import { Effect } from "effect"
import {
  App,
  CommandMenu,
  type AppBridge,
} from "../src/app.tsx"
import type { AppCommand } from "../src/services/app-state.ts"

const commands: Array<AppCommand> = []
const bridge: AppBridge = {
  projectPaths: () => [
    { path: "src/", isDirectory: true },
    { path: "src/app.tsx", isDirectory: false },
  ],
  initial: {
    cwd: "/work/pi",
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
    commands: [
      {
        name: "session",
        description: "Show session info and stats",
        source: "builtin",
      },
    ],
  },
  subscribe: () => () => undefined,
  dispatch: (command) => {
    commands.push(command)
  },
  quit: () => undefined,
}

test("renders chat and sends input", () => {
  commands.length = 0
  return Effect.runPromise(
    Effect.acquireUseRelease(
      Effect.tryPromise(() =>
        testRender(() => <App bridge={bridge} />, {
          width: 100,
          height: 30,
        }),
      ),
      (setup) =>
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
            setup.mockInput.typeText("run tests"),
          )
          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())

          expect(commands).toEqual([
            { _tag: "Prompt", text: "run tests" },
          ])
        }),
      (setup) => Effect.sync(() => setup.renderer.destroy()),
    ),
  )
})

test("resolves an extension selection dialog", () => {
  commands.length = 0
  const dialogBridge: AppBridge = {
    ...bridge,
    initial: {
      ...bridge.initial,
      phase: "running",
      dialog: {
        id: 7,
        kind: "select",
        title: "Allow write?",
        message: "/tmp/output",
        options: ["Allow once", "Deny"],
        placeholder: undefined,
      },
    },
  }

  return Effect.runPromise(
    Effect.acquireUseRelease(
      Effect.tryPromise(() =>
        testRender(() => <App bridge={dialogBridge} />, {
          width: 80,
          height: 24,
        }),
      ),
      (setup) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() => setup.renderOnce())
          yield* Effect.tryPromise(() => setup.flush())
          expect(setup.captureCharFrame()).toContain("Allow write?")

          setup.mockInput.pressArrow("down")
          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())

          expect(commands).toContainEqual({
            _tag: "ResolveDialog",
            id: 7,
            value: "Deny",
          })
        }),
      (setup) => Effect.sync(() => setup.renderer.destroy()),
    ),
  )
})

test("completes slash commands while typing", () => {
  commands.length = 0
  const autocompleteBridge: AppBridge = {
    ...bridge,
    initial: {
      ...bridge.initial,
      commands: [
        ...bridge.initial.commands,
        {
          name: "resume",
          description: "Resume a saved session",
          source: "builtin",
        },
      ],
    },
  }

  return Effect.runPromise(
    Effect.acquireUseRelease(
      Effect.tryPromise(() =>
        testRender(() => <App bridge={autocompleteBridge} />, {
          width: 100,
          height: 30,
        }),
      ),
      (setup) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() =>
            setup.mockInput.typeText("/res"),
          )
          yield* Effect.tryPromise(() => setup.flush())
          expect(setup.captureCharFrame()).toContain(
            "/resume — Resume a saved session",
          )

          setup.mockInput.pressTab()
          yield* Effect.tryPromise(() => setup.flush())
          expect(setup.captureCharFrame()).toContain("/resume")

          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(commands).toContainEqual({
            _tag: "Prompt",
            text: "/resume",
          })
        }),
      (setup) => Effect.sync(() => setup.renderer.destroy()),
    ),
  )
})

test("completes file mentions while typing", () => {
  commands.length = 0
  return Effect.runPromise(
    Effect.acquireUseRelease(
      Effect.tryPromise(() =>
        testRender(() => <App bridge={bridge} />, {
          width: 100,
          height: 30,
        }),
      ),
      (setup) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() =>
            setup.mockInput.typeText("check @app"),
          )
          yield* Effect.tryPromise(() => setup.flush())
          expect(setup.captureCharFrame()).toContain("src/app.tsx")

          setup.mockInput.pressTab()
          yield* Effect.tryPromise(() => setup.flush())
          expect(setup.captureCharFrame()).toContain(
            "check @src/app.tsx",
          )

          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(commands).toContainEqual({
            _tag: "Prompt",
            text: "check @src/app.tsx",
          })
        }),
      (setup) => Effect.sync(() => setup.renderer.destroy()),
    ),
  )
})

test("hides login secrets while resolving them", () => {
  commands.length = 0
  const secretBridge: AppBridge = {
    ...bridge,
    initial: {
      ...bridge.initial,
      phase: "running",
      dialog: {
        id: 9,
        kind: "secret",
        title: "Enter API key",
        message: undefined,
        options: [],
        placeholder: "key",
      },
    },
  }

  return Effect.runPromise(
    Effect.acquireUseRelease(
      Effect.tryPromise(() =>
        testRender(() => <App bridge={secretBridge} />, {
          width: 80,
          height: 24,
        }),
      ),
      (setup) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() =>
            setup.mockInput.typeText("private-key"),
          )
          yield* Effect.tryPromise(() => setup.flush())
          const frame = setup.captureCharFrame()
          expect(frame).toContain("input hidden")
          expect(frame).not.toContain("private-key")

          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(commands).toContainEqual({
            _tag: "ResolveDialog",
            id: 9,
            value: "private-key",
          })
        }),
      (setup) => Effect.sync(() => setup.renderer.destroy()),
    ),
  )
})

test("chooses a command from the slash menu", () => {
  let selected: string | undefined
  return Effect.runPromise(
    Effect.acquireUseRelease(
      Effect.tryPromise(() =>
        testRender(() => (
          <CommandMenu
            commands={bridge.initial.commands}
            resolve={(name) => {
              selected = name
            }}
          />
        ), {
          width: 80,
          height: 24,
        }),
      ),
      (setup) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() => setup.renderOnce())
          expect(setup.captureCharFrame()).toContain("Commands")

          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(selected).toBe("session")
        }),
      (setup) => Effect.sync(() => setup.renderer.destroy()),
    ),
  )
})
