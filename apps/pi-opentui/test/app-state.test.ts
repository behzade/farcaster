import {
  type AgentSessionEvent,
  type ExtensionUIContext,
} from "@earendil-works/pi-coding-agent"
import { expect, test } from "bun:test"
import { Effect, Layer, Stream } from "effect"
import {
  AppState,
  AppStateLive,
} from "../src/services/app-state.ts"
import {
  PiSession,
  PiSessionError,
} from "../src/services/pi-session.ts"

test("folds session events and commands into app state", () => {
  let emit: ((event: AgentSessionEvent) => void) | undefined
  let extensionUi: ExtensionUIContext | undefined
  let prompt = ""
  let promptCalls = 0
  let finishPrompt: (() => void) | undefined
  let failPrompt: (() => void) | undefined
  let compactInstructions: string | undefined

  const events = Stream.asyncPush<AgentSessionEvent>((push) =>
    Effect.sync(() => {
      emit = (event) => {
        push.single(event)
      }
    }),
  )

  const pi = Layer.succeed(PiSession, {
    cwd: "/work",
    activeTools: ["sandbox"],
    extensionPaths: ["/agent/extensions/sandbox"],
    extensionErrors: [],
    events,
    commands: Effect.succeed([
      {
        name: "review",
        description: "Review changes",
        source: "extension" as const,
        sourceInfo: {
          path: "/agent/extensions/review.ts",
          source: "review",
          scope: "user" as const,
          origin: "top-level" as const,
        },
      },
      {
        name: "session",
        description: "Extension conflict",
        source: "extension" as const,
        sourceInfo: {
          path: "/agent/extensions/session.ts",
          source: "session",
          scope: "user" as const,
          origin: "top-level" as const,
        },
      },
    ]),
    sessionStats: Effect.succeed({
      sessionFile: undefined,
      sessionId: "session-1",
      userMessages: 1,
      assistantMessages: 1,
      toolCalls: 0,
      toolResults: 0,
      totalMessages: 2,
      tokens: {
        input: 10,
        output: 20,
        cacheRead: 0,
        cacheWrite: 0,
        total: 30,
      },
      cost: 0.01,
    }),
    prompt: (text) =>
      Effect.async<void, PiSessionError>((resume) => {
        prompt = text
        promptCalls += 1
        finishPrompt = () => resume(Effect.void)
        failPrompt = () =>
          resume(
            Effect.fail(
              new PiSessionError({
                operation: "prompt",
                cause: new Error("Request was aborted"),
              }),
            ),
          )
      }),
    compact: (instructions) =>
      Effect.sync(() => {
        compactInstructions = instructions
      }),
    abort: Effect.sync(() => {
      failPrompt?.()
    }),
    bindExtensions: (ui) =>
      Effect.sync(() => {
        extensionUi = ui
      }),
  })
  const appLayer = AppStateLive.pipe(Layer.provide(pi))

  const program = Effect.scoped(
    Effect.gen(function* () {
      const app = yield* AppState

      while (emit === undefined) {
        yield* Effect.yieldNow()
      }
      emit({ type: "agent_settled" })

      let snapshot = yield* app.get
      while (snapshot.eventCount === 0) {
        yield* Effect.yieldNow()
        snapshot = yield* app.get
      }

      expect(snapshot.lastEvent).toBe("agent_settled")
      expect(snapshot.commands.map((command) => command.name)).toContain(
        "review",
      )
      expect(
        snapshot.commands.filter((command) => command.name === "session"),
      ).toEqual([
        {
          name: "session",
          description: "Show session info and stats",
          source: "builtin",
        },
      ])
      extensionUi?.notify("sandbox ready")
      while (
        !(yield* app.get).transcript.rows.some(
          (row) => row.content === "sandbox ready",
        )
      ) {
        yield* Effect.yieldNow()
      }

      const selection = extensionUi?.select("Allow write?", [
        "Allow once",
        "Deny",
      ])
      let dialog = (yield* app.get).dialog
      while (dialog === undefined) {
        yield* Effect.yieldNow()
        dialog = (yield* app.get).dialog
      }
      yield* app.dispatch({
        _tag: "ResolveDialog",
        id: dialog.id,
        value: "Allow once",
      })
      expect(
        yield* Effect.promise(() =>
          selection ?? Promise.resolve(undefined),
        ),
      ).toBe("Allow once")

      yield* app.dispatch({ _tag: "Prompt", text: "  test prompt  " })
      yield* app.dispatch({ _tag: "Prompt", text: "second prompt" })
      while (prompt.length === 0) {
        yield* Effect.yieldNow()
      }
      expect(prompt).toBe("test prompt")
      expect((yield* app.get).phase).toBe("running")
      expect(promptCalls).toBe(1)
      finishPrompt?.()
      while ((yield* app.get).phase !== "ready") {
        yield* Effect.yieldNow()
      }
      expect((yield* app.get).phase).toBe("ready")

      prompt = ""
      yield* app.dispatch({ _tag: "Prompt", text: "stop me" })
      while (prompt !== "stop me") {
        yield* Effect.yieldNow()
      }
      yield* app.dispatch({ _tag: "Abort" })
      while ((yield* app.get).phase !== "ready") {
        yield* Effect.yieldNow()
      }
      expect((yield* app.get).error).toBeUndefined()
      expect(
        (yield* app.get).transcript.rows.some((row) =>
          row.content.includes("Request was aborted"),
        ),
      ).toBe(false)

      const callsBeforeCommands = promptCalls
      yield* app.dispatch({ _tag: "Prompt", text: "/session" })
      while (
        !(yield* app.get).transcript.rows.some((row) =>
          row.content.includes("Session session-1"),
        )
      ) {
        yield* Effect.yieldNow()
      }
      expect(promptCalls).toBe(callsBeforeCommands)

      yield* app.dispatch({ _tag: "Prompt", text: "/missing" })
      while (
        !(yield* app.get).transcript.rows.some(
          (row) => row.content === "Unknown command: /missing",
        )
      ) {
        yield* Effect.yieldNow()
      }
      expect(promptCalls).toBe(callsBeforeCommands)

      yield* app.dispatch({
        _tag: "Prompt",
        text: "/compact keep decisions",
      })
      while (compactInstructions === undefined) {
        yield* Effect.yieldNow()
      }
      expect(compactInstructions).toBe("keep decisions")
    }),
  ).pipe(Effect.provide(appLayer))

  return Effect.runPromise(program)
})
