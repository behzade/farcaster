import {
  type AgentSessionEvent,
} from "@earendil-works/pi-coding-agent"
import { describe, expect, test } from "bun:test"
import {
  Effect,
  Fiber,
  Layer,
  Option,
  Stream,
} from "effect"
import { AppConfig } from "../src/services/app-config.ts"
import {
  PiSession,
  makePiSessionLayer,
  type OpenedPiSession,
} from "../src/services/pi-session.ts"

describe("PiSession", () => {
  test("delivers events and cleans up its listener and session", () => {
    let listener: ((event: AgentSessionEvent) => void) | undefined
    let disposed = false
    let shutDown = false
    let unsubscribed = false
    let openedWithSavedSessions: boolean | undefined
    let listedSessions = 0
    let newSessions = 0
    let resumedPath: string | undefined

    const open = (
      _cwd: string,
      saveSessions: boolean,
    ): Promise<OpenedPiSession> => {
      openedWithSavedSessions = saveSessions
      return Promise.resolve({
        getCommands: () => [],
        listSessions: () => {
          listedSessions += 1
          return Promise.resolve([])
        },
        getMessages: () => [{ role: "user", content: "current" }],
        newSession: () => {
          newSessions += 1
          return Promise.resolve([])
        },
        resume: (path) => {
          resumedPath = path
          return Promise.resolve([
            { role: "user", content: "restored" },
          ])
        },
        shutdown: () => {
          shutDown = true
          return Promise.resolve()
        },
        session: {
          subscribe: (next: (event: AgentSessionEvent) => void) => {
            listener = next
            return () => {
              unsubscribed = true
            }
          },
          dispose: () => {
            disposed = true
          },
          getActiveToolNames: () => ["read", "sandbox"],
          getSessionStats: () => ({
            sessionFile: undefined,
            sessionId: "session-1",
            userMessages: 0,
            assistantMessages: 0,
            toolCalls: 0,
            toolResults: 0,
            totalMessages: 0,
            tokens: {
              input: 0,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
              total: 0,
            },
            cost: 0,
          }),
          prompt: () => Promise.resolve(),
          compact: () => Promise.resolve(),
          abort: () => Promise.resolve(),
          bindExtensions: () => Promise.resolve(),
        },
        extensionsResult: {
          extensions: [
            { path: "/agent/extensions/sandbox" },
          ],
          errors: [],
        },
      })
    }

    const config = Layer.succeed(AppConfig, {
      cwd: "/work",
      saveSessions: false,
    })
    const session = makePiSessionLayer(open).pipe(
      Layer.provide(config),
    )

    const program = Effect.scoped(
      Effect.gen(function* () {
        const pi = yield* PiSession
        const waitForEvent = yield* Stream.runHead(pi.events).pipe(
          Effect.fork,
        )

        while (listener === undefined) {
          yield* Effect.yieldNow()
        }
        listener({ type: "agent_settled" })

        const event = yield* Fiber.join(waitForEvent)
        expect(Option.getOrThrow(event).type).toBe("agent_settled")
        expect(pi.activeTools).toEqual(["read", "sandbox"])
        expect(pi.extensionPaths).toEqual([
          "/agent/extensions/sandbox",
        ])
        expect(yield* pi.sessions).toEqual([])
        expect(yield* pi.messages).toEqual([
          { role: "user", content: "current" },
        ])
        expect(yield* pi.newSession).toEqual([])
        expect(yield* pi.resume("/sessions/old.jsonl")).toEqual([
          { role: "user", content: "restored" },
        ])
        expect(openedWithSavedSessions).toBe(false)
        expect(listedSessions).toBe(1)
        expect(newSessions).toBe(1)
        expect(resumedPath).toBe("/sessions/old.jsonl")
      }),
    ).pipe(Effect.provide(session))

    return Effect.runPromise(program).then(() => {
      expect(unsubscribed).toBe(true)
      expect(shutDown).toBe(true)
      expect(disposed).toBe(true)
    })
  })
})
