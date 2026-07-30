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

    const open = (): Promise<OpenedPiSession> =>
      Promise.resolve({
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
          prompt: () => Promise.resolve(),
          abort: () => Promise.resolve(),
        },
        extensionsResult: {
          extensions: [
            { path: "/agent/extensions/sandbox" },
          ],
          errors: [],
        },
      })

    const config = Layer.succeed(AppConfig, { cwd: "/work" })
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
      }),
    ).pipe(Effect.provide(session))

    return Effect.runPromise(program).then(() => {
      expect(unsubscribed).toBe(true)
      expect(shutDown).toBe(true)
      expect(disposed).toBe(true)
    })
  })
})
