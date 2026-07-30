import {
  type AgentSessionEvent,
} from "@earendil-works/pi-coding-agent"
import { expect, test } from "bun:test"
import { Effect, Layer, Stream } from "effect"
import {
  AppState,
  AppStateLive,
} from "../src/services/app-state.ts"
import { PiSession } from "../src/services/pi-session.ts"

test("folds session events and commands into app state", () => {
  let emit: ((event: AgentSessionEvent) => void) | undefined
  let prompt = ""

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
    prompt: (text) =>
      Effect.sync(() => {
        prompt = text
      }),
    abort: Effect.void,
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
      yield* app.dispatch({ _tag: "Prompt", text: "  test prompt  " })
      expect(prompt).toBe("test prompt")
      expect((yield* app.get).phase).toBe("ready")
    }),
  ).pipe(Effect.provide(appLayer))

  return Effect.runPromise(program)
})
