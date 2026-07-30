import { BunRuntime } from "@effect/platform-bun"
import { render } from "@opentui/solid"
import {
  Data,
  Deferred,
  Effect,
  Fiber,
  Runtime,
  Stream,
} from "effect"
import { App, type AppBridge } from "./app.tsx"
import { AppLive } from "./runtime.ts"
import { AppState } from "./services/app-state.ts"
import { UiRenderer } from "./services/ui-renderer.ts"

class RenderError extends Data.TaggedError("RenderError")<{
  readonly cause: unknown
}> {}

const program = Effect.scoped(
  Effect.gen(function* () {
    const app = yield* AppState
    const ui = yield* UiRenderer
    const initial = yield* app.get
    const scope = yield* Effect.scope
    const runtime = yield* Effect.runtime<never>()
    const stopped = yield* Deferred.make<void>()
    const runFork = Runtime.runFork(runtime)

    const bridge: AppBridge = {
      initial,
      subscribe: (listener) => {
        const fiber = runFork(
          Stream.runForEach(app.changes, (snapshot) =>
            Effect.sync(() => listener(snapshot)),
          ),
          { scope },
        )

        return () => {
          runFork(Fiber.interrupt(fiber), { scope })
        }
      },
      dispatch: (command) => {
        runFork(app.dispatch(command), { scope })
      },
      quit: () => {
        runFork(Deferred.succeed(stopped, undefined), { scope })
      },
    }

    yield* Effect.tryPromise({
      try: () => render(() => <App bridge={bridge} />, ui.renderer),
      catch: (cause) => new RenderError({ cause }),
    })
    yield* Deferred.await(stopped)
  }),
).pipe(Effect.provide(AppLive))

BunRuntime.runMain(program)
