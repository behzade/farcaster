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
      setAuthNotice: () => Effect.void,
      setTitle: () => Effect.void,
    })
    yield* pi.bindExtensions(extensionUi.context, () => undefined)
    const sessionBefore = yield* pi.sessionStats
    const replacementMessages = yield* pi.newSession
    const sessionAfter = yield* pi.sessionStats
    const modelState = yield* pi.modelState
    const models = yield* pi.models
    const authProviders = yield* pi.authProviders
    const webSearchPresentation = pi.presentExtensionTool({
      toolCallId: "smoke-web-search",
      toolName: "web_search",
      args: { query: "OpenTUI smoke" },
      result: undefined,
      pending: false,
      isError: false,
    })
    const modelsByProvider = Object.fromEntries(
      Object.entries(
        models.reduce<Record<string, number>>((counts, model) => {
          counts[model.provider] = (counts[model.provider] ?? 0) + 1
          return counts
        }, {}),
      ).toSorted(([left], [right]) => left.localeCompare(right)),
    )
    if (sessionAfter.sessionId === sessionBefore.sessionId) {
      return yield* Effect.fail(
        new Error("Pi kept the old session after replacement"),
      )
    }
    if (
      pi.activeTools.includes("web_search") &&
      !webSearchPresentation?.call?.includes("OpenTUI smoke")
    ) {
      return yield* Effect.fail(
        new Error("The web search extension renderer was not available"),
      )
    }

    yield* Console.log(
      JSON.stringify(
        {
          cwd: pi.cwd,
          activeTools: pi.activeTools,
          extensionPaths: pi.extensionPaths,
          extensionErrors: pi.extensionErrors,
          model: modelState.selected,
          thinkingLevel: modelState.thinkingLevel,
          availableModels: models.length,
          modelsByProvider,
          loginMethods: authProviders.length,
          hasOpenCodeGoLogin: authProviders.some(
            (provider) =>
              provider.id === "opencode-go" &&
              provider.type === "api_key" &&
              provider.interactive,
          ),
          hasWebSearchPresentation:
            webSearchPresentation?.call?.includes("OpenTUI smoke") ?? false,
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
