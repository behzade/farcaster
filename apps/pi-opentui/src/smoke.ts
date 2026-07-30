import { BunContext, BunRuntime } from "@effect/platform-bun"
import { Console, Effect, Layer } from "effect"
import { AppConfig } from "./services/app-config.ts"
import {
  PiSession,
  makePiSessionLayer,
} from "./services/pi-session.ts"

const SmokeSessionLive = makePiSessionLayer().pipe(
  Layer.provide(
    Layer.succeed(AppConfig, {
      cwd: process.cwd(),
      saveSessions: false,
    }),
  ),
)

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
  Effect.provide(SmokeSessionLive),
  Effect.provide(BunContext.layer),
)

BunRuntime.runMain(smoke)
