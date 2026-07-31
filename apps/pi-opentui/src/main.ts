import { BunContext, BunRuntime } from "@effect/platform-bun"
import { Data, Deferred, Effect, Fiber, Layer, Runtime, Stream } from "effect"
import { mountApp } from "./opentui/app-view.ts"
import {
  UiRenderer,
  UiRendererLive,
} from "./opentui/ui-renderer.ts"
import { AppLive } from "./runtime.ts"
import { AppState } from "./services/app-state.ts"
import {
  listProjectPaths,
  type ProjectPath,
} from "./services/project-paths.ts"
import {
  PasteService,
  PasteServiceLive,
} from "./services/paste-service.ts"
import {
  Keybindings,
} from "./services/keybindings.ts"
import type { AppClient } from "./ui/app-client.ts"

class RenderError extends Data.TaggedError("RenderError")<{
  readonly cause: unknown
}> {}

const MainLive = Layer.mergeAll(
  AppLive,
  UiRendererLive,
  PasteServiceLive,
)

const program = Effect.scoped(
  Effect.gen(function* () {
    const app = yield* AppState
    const ui = yield* UiRenderer
    const paste = yield* PasteService
    const keybindings = yield* Keybindings
    const initial = yield* app.get
    const scope = yield* Effect.scope
    const runtime = yield* Effect.runtime<never>()
    const stopped = yield* Deferred.make<void>()
    const runFork = Runtime.runFork(runtime)
    let projectPaths: ReadonlyArray<ProjectPath> = []

    const client: AppClient = {
      initial,
      projectPaths: () => projectPaths,
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
      resolvePaste: (request, accept) => {
        runFork(
          paste.resolve(request).pipe(
            Effect.match({
              onFailure: () => undefined,
              onSuccess: (insertion) => insertion,
            }),
            Effect.tap((insertion) =>
              Effect.sync(() => accept(insertion)),
            ),
          ),
          { scope },
        )
      },
      quit: () => {
        runFork(Deferred.succeed(stopped, undefined), { scope })
      },
    }

    yield* Effect.acquireRelease(
      Effect.try({
        try: () => mountApp(ui.renderer, client, keybindings),
        catch: (cause) => new RenderError({ cause }),
      }),
      (view) => Effect.sync(() => view.destroy()),
    )
    yield* listProjectPaths(initial.cwd).pipe(
      Effect.catchAll(() => Effect.succeed([])),
      Effect.tap((entries) =>
        Effect.sync(() => {
          projectPaths = entries
        }),
      ),
      Effect.forkScoped,
    )
    yield* Deferred.await(stopped)
  }),
).pipe(
  Effect.provide(MainLive),
  Effect.provide(BunContext.layer),
)

BunRuntime.runMain(program)
