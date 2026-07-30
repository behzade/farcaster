import { testRender } from "@opentui/solid"
import { expect, test } from "bun:test"
import { Effect } from "effect"
import { App, type AppBridge } from "../src/app.tsx"
import type { AppCommand } from "../src/services/app-state.ts"

const commands: Array<AppCommand> = []
const bridge: AppBridge = {
  initial: {
    cwd: "/work/pi",
    phase: "ready",
    activeTools: ["read", "sandbox"],
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
    statuses: {},
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
