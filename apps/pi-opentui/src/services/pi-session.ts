import {
  SessionManager,
  createAgentSession,
  type AgentSessionEvent,
  type ExtensionError,
  type ExtensionUIContext,
} from "@earendil-works/pi-coding-agent"
import { Context, Data, Effect, Layer, Stream } from "effect"
import { AppConfig } from "./app-config.ts"

export interface ExtensionLoadError {
  readonly path: string
  readonly error: string
}

export interface PiSessionShape {
  readonly cwd: string
  readonly activeTools: ReadonlyArray<string>
  readonly extensionPaths: ReadonlyArray<string>
  readonly extensionErrors: ReadonlyArray<ExtensionLoadError>
  readonly events: Stream.Stream<AgentSessionEvent>
  readonly bindExtensions: (
    uiContext: ExtensionUIContext,
    onError: (error: ExtensionError) => void,
  ) => Effect.Effect<void, PiSessionError>
  readonly prompt: (text: string) => Effect.Effect<void, PiSessionError>
  readonly abort: Effect.Effect<void, PiSessionError>
}

export class PiSession extends Context.Tag("pi-opentui/PiSession")<
  PiSession,
  PiSessionShape
>() {}

export class PiSessionError extends Data.TaggedError("PiSessionError")<{
  readonly operation: "open" | "bind" | "prompt" | "abort" | "shutdown"
  readonly cause: unknown
}> {}

export interface OpenedPiSession {
  readonly shutdown: () => Promise<void>
  readonly session: {
    readonly subscribe: (
      listener: (event: AgentSessionEvent) => void,
    ) => () => void
    readonly dispose: () => void
    readonly getActiveToolNames: () => Array<string>
    readonly prompt: (text: string) => Promise<void>
    readonly abort: () => Promise<void>
    readonly bindExtensions: (bindings: {
      readonly uiContext: ExtensionUIContext
      readonly mode: "tui"
      readonly onError: (error: ExtensionError) => void
    }) => Promise<void>
  }
  readonly extensionsResult: {
    readonly extensions: ReadonlyArray<{ readonly path: string }>
    readonly errors: ReadonlyArray<ExtensionLoadError>
  }
}

export type OpenPiSession = (
  cwd: string,
) => Promise<OpenedPiSession>

const openPiSession: OpenPiSession = (cwd) => {
  const sessionManager = SessionManager.inMemory(cwd)
  return createAgentSession({ cwd, sessionManager }).then((result) => ({
    session: result.session,
    extensionsResult: result.extensionsResult,
    shutdown: () =>
      result.session.extensionRunner
        .emit({ type: "session_shutdown", reason: "quit" })
        .then(() => undefined),
  }))
}

const sdkCall = (
  operation: "prompt" | "abort",
  call: () => Promise<void>,
): Effect.Effect<void, PiSessionError> =>
  Effect.tryPromise({
    try: call,
    catch: (cause) => new PiSessionError({ operation, cause }),
  })

const sessionEvents = (
  session: OpenedPiSession["session"],
): Stream.Stream<AgentSessionEvent> =>
  Stream.asyncPush(
    (emit) =>
      Effect.acquireRelease(
        Effect.sync(() =>
          session.subscribe((event) => {
            emit.single(event)
          }),
        ),
        (unsubscribe) => Effect.sync(unsubscribe),
      ),
    { bufferSize: "unbounded" },
  )

export const makePiSessionLayer = (
  open: OpenPiSession = openPiSession,
): Layer.Layer<PiSession, PiSessionError, AppConfig> =>
  Layer.scoped(
    PiSession,
    Effect.gen(function* () {
      const config = yield* AppConfig
      const result = yield* Effect.acquireRelease(
        Effect.tryPromise({
          try: () => open(config.cwd),
          catch: (cause) =>
            new PiSessionError({ operation: "open", cause }),
        }),
        ({ session, shutdown }) =>
          Effect.tryPromise({
            try: shutdown,
            catch: (cause) =>
              new PiSessionError({ operation: "shutdown", cause }),
          }).pipe(
            Effect.catchAll((error) =>
              Effect.logWarning(
                `Pi extension shutdown failed: ${String(error.cause)}`,
              ),
            ),
            Effect.ensuring(Effect.sync(() => session.dispose())),
          ),
      )

      return {
        cwd: config.cwd,
        activeTools: result.session.getActiveToolNames().toSorted(),
        extensionPaths: result.extensionsResult.extensions
          .map((extension) => extension.path)
          .toSorted(),
        extensionErrors: result.extensionsResult.errors,
        events: sessionEvents(result.session),
        bindExtensions: (uiContext, onError) =>
          Effect.tryPromise({
            try: () =>
              result.session.bindExtensions({
                uiContext,
                mode: "tui",
                onError,
              }),
            catch: (cause) =>
              new PiSessionError({ operation: "bind", cause }),
          }),
        prompt: (text) =>
          sdkCall("prompt", () => result.session.prompt(text)),
        abort: sdkCall("abort", () => result.session.abort()),
      }
    }),
  )
