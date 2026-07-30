import { BunContext, BunRuntime } from "@effect/platform-bun"
import { Console, Effect, Layer } from "effect"
import { AppConfig } from "./services/app-config.ts"
import { makeExtensionUi } from "./services/extension-ui.ts"
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
    const extensionUi = yield* makeExtensionUi({
      setDialog: () => Effect.void,
      notify: () => Effect.void,
      setStatus: () => Effect.void,
    })
    yield* pi.bindExtensions(extensionUi.context, () => undefined)
    const sessionBefore = yield* pi.sessionStats
    const replacementMessages = yield* pi.newSession
    const sessionAfter = yield* pi.sessionStats
    if (sessionAfter.sessionId === sessionBefore.sessionId) {
      return yield* Effect.fail(
        new Error("Pi kept the old session after replacement"),
      )
    }

    yield* Console.log(
      JSON.stringify(
        {
          cwd: pi.cwd,
          activeTools: pi.activeTools,
          extensionPaths: pi.extensionPaths,
          extensionErrors: pi.extensionErrors,
          sessionReplacement: {
            before: sessionBefore.sessionId,
            after: sessionAfter.sessionId,
            messages: replacementMessages.length,
          },
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
