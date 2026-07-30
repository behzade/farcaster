import {
  Context,
  Effect,
  Layer,
  Stream,
  SubscriptionRef,
} from "effect"
import {
  PiSession,
  type ExtensionLoadError,
  type PiSessionError,
} from "./pi-session.ts"

export type AppPhase = "ready" | "running" | "stopping" | "error"

export interface AppSnapshot {
  readonly cwd: string
  readonly phase: AppPhase
  readonly activeTools: ReadonlyArray<string>
  readonly extensionPaths: ReadonlyArray<string>
  readonly extensionErrors: ReadonlyArray<ExtensionLoadError>
  readonly eventCount: number
  readonly lastEvent: string | undefined
  readonly error: string | undefined
}

export type AppCommand =
  | { readonly _tag: "Prompt"; readonly text: string }
  | { readonly _tag: "Abort" }

export interface AppStateShape {
  readonly get: Effect.Effect<AppSnapshot>
  readonly changes: Stream.Stream<AppSnapshot>
  readonly dispatch: (command: AppCommand) => Effect.Effect<void>
}

export class AppState extends Context.Tag("pi-opentui/AppState")<
  AppState,
  AppStateShape
>() {}

const errorText = (error: PiSessionError): string => {
  const cause =
    error.cause instanceof Error ? error.cause.message : String(error.cause)
  return `${error.operation}: ${cause}`
}

export const AppStateLive: Layer.Layer<AppState, never, PiSession> =
  Layer.scoped(
    AppState,
    Effect.gen(function* () {
      const pi = yield* PiSession
      const state = yield* SubscriptionRef.make<AppSnapshot>({
        cwd: pi.cwd,
        phase: "ready",
        activeTools: pi.activeTools,
        extensionPaths: pi.extensionPaths,
        extensionErrors: pi.extensionErrors,
        eventCount: 0,
        lastEvent: undefined,
        error: undefined,
      })

      yield* Stream.runForEach(pi.events, (event) =>
        SubscriptionRef.update(state, (snapshot) => ({
          ...snapshot,
          eventCount: snapshot.eventCount + 1,
          lastEvent: event.type,
        })),
      ).pipe(Effect.forkScoped)

      const runPrompt = (text: string): Effect.Effect<void> =>
        Effect.gen(function* () {
          const prompt = text.trim()
          if (prompt.length === 0) return

          yield* SubscriptionRef.update(state, (snapshot) => ({
            ...snapshot,
            phase: "running" as const,
            error: undefined,
          }))
          yield* pi.prompt(prompt)
          yield* SubscriptionRef.update(state, (snapshot) => ({
            ...snapshot,
            phase: "ready" as const,
          }))
        }).pipe(
          Effect.catchAll((error) =>
            SubscriptionRef.update(state, (snapshot) => ({
              ...snapshot,
              phase: "error" as const,
              error: errorText(error),
            })),
          ),
        )

      const abort = pi.abort.pipe(
        Effect.zipLeft(
          SubscriptionRef.update(state, (snapshot) => ({
            ...snapshot,
            phase: "ready" as const,
          })),
        ),
        Effect.catchAll((error) =>
          SubscriptionRef.update(state, (snapshot) => ({
            ...snapshot,
            phase: "error" as const,
            error: errorText(error),
          })),
        ),
      )

      return {
        get: SubscriptionRef.get(state),
        changes: state.changes,
        dispatch: (command) => {
          switch (command._tag) {
            case "Prompt":
              return runPrompt(command.text)
            case "Abort":
              return SubscriptionRef.update(state, (snapshot) => ({
                ...snapshot,
                phase: "stopping" as const,
                error: undefined,
              })).pipe(Effect.zipRight(abort))
          }
        },
      }
    }),
  )
