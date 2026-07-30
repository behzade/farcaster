import { BunContext, BunRuntime } from "@effect/platform-bun"
import { Console, Effect } from "effect"
import { PiSessionLive } from "./runtime.ts"
import { PiSession } from "./services/pi-session.ts"

const smoke = Effect.scoped(
  Effect.gen(function* () {
    const pi = yield* PiSession
    yield* Console.log(
      JSON.stringify(
        {
          cwd: pi.cwd,
          activeTools: pi.activeTools,
          extensionPaths: pi.extensionPaths,
          extensionErrors: pi.extensionErrors,
        },
        null,
        2,
      ),
    )
  }),
).pipe(
  Effect.provide(PiSessionLive),
  Effect.provide(BunContext.layer),
)

BunRuntime.runMain(smoke)
