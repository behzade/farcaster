import {
  createAgentSessionFromServices,
  createAgentSessionRuntime,
  createAgentSessionServices,
  getAgentDir,
  SessionManager,
  type AgentSessionEvent,
  type ExtensionError,
  type ExtensionUIContext,
  type SessionInfo,
  type SessionStats,
  type SlashCommandInfo,
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
  readonly commands: Effect.Effect<ReadonlyArray<SlashCommandInfo>>
  readonly sessionStats: Effect.Effect<SessionStats>
  readonly sessions: Effect.Effect<ReadonlyArray<SessionInfo>, PiSessionError>
  readonly messages: Effect.Effect<ReadonlyArray<unknown>>
  readonly bindExtensions: (
    uiContext: ExtensionUIContext,
    onError: (error: ExtensionError) => void,
  ) => Effect.Effect<void, PiSessionError>
  readonly prompt: (text: string) => Effect.Effect<void, PiSessionError>
  readonly compact: (
    instructions?: string,
  ) => Effect.Effect<void, PiSessionError>
  readonly newSession: Effect.Effect<ReadonlyArray<unknown>, PiSessionError>
  readonly resume: (
    path: string,
  ) => Effect.Effect<ReadonlyArray<unknown>, PiSessionError>
  readonly abort: Effect.Effect<void, PiSessionError>
}

export class PiSession extends Context.Tag("pi-opentui/PiSession")<
  PiSession,
  PiSessionShape
>() {}

export class PiSessionError extends Data.TaggedError("PiSessionError")<{
  readonly operation:
    | "open"
    | "bind"
    | "prompt"
    | "compact"
    | "list"
    | "new"
    | "resume"
    | "abort"
    | "shutdown"
  readonly cause: unknown
}> {}

export interface OpenedPiSession {
  readonly shutdown: () => Promise<void>
  readonly getCommands: () => Array<SlashCommandInfo>
  readonly listSessions: () => Promise<Array<SessionInfo>>
  readonly getMessages: () => ReadonlyArray<unknown>
  readonly newSession: () => Promise<ReadonlyArray<unknown>>
  readonly resume: (path: string) => Promise<ReadonlyArray<unknown>>
  readonly session: {
    readonly subscribe: (
      listener: (event: AgentSessionEvent) => void,
    ) => () => void
    readonly dispose: () => void
    readonly getActiveToolNames: () => Array<string>
    readonly getSessionStats: () => SessionStats
    readonly prompt: (text: string) => Promise<void>
    readonly compact: (instructions?: string) => Promise<unknown>
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
  saveSessions: boolean,
) => Promise<OpenedPiSession>

const openPiSession: OpenPiSession = (cwd, saveSessions) => {
  const sessionManager = saveSessions
    ? SessionManager.create(cwd)
    : SessionManager.inMemory(cwd)
  const agentDir = getAgentDir()
  const createRuntime = (options: {
    cwd: string
    agentDir: string
    sessionManager: SessionManager
    sessionStartEvent?: Parameters<
      typeof createAgentSessionFromServices
    >[0]["sessionStartEvent"]
  }) =>
    Effect.runPromise(
      Effect.gen(function* () {
        const services = yield* Effect.tryPromise(() =>
          createAgentSessionServices({
            cwd: options.cwd,
            agentDir: options.agentDir,
          }),
        )
        const created = yield* Effect.tryPromise(() =>
          createAgentSessionFromServices({
            services,
            sessionManager: options.sessionManager,
            ...(options.sessionStartEvent === undefined
              ? {}
              : { sessionStartEvent: options.sessionStartEvent }),
          }),
        )
        return {
          ...created,
          services,
          diagnostics: services.diagnostics,
        }
      }),
    )

  return Effect.runPromise(
    Effect.gen(function* () {
      const runtime = yield* Effect.tryPromise(() =>
        createAgentSessionRuntime(createRuntime, {
          cwd,
          agentDir,
          sessionManager,
        }),
      )
      const listeners = new Set<(event: AgentSessionEvent) => void>()
      let unsubscribe: () => void = () => undefined
      let bindings:
        | {
            readonly uiContext: ExtensionUIContext
            readonly onError: (error: ExtensionError) => void
          }
        | undefined

      const bindCurrent = Effect.gen(function* () {
        yield* Effect.sync(() => {
          unsubscribe()
          unsubscribe = runtime.session.subscribe((event) => {
            for (const listener of listeners) listener(event)
          })
        })
        if (bindings !== undefined) {
          yield* Effect.tryPromise(() =>
            runtime.session.bindExtensions({
              ...bindings,
              mode: "tui",
              commandContextActions: {
                waitForIdle: () => runtime.session.waitForIdle(),
                newSession: (options) => runtime.newSession(options),
                fork: (entryId, options) =>
                  runtime.fork(entryId, options),
                navigateTree: (targetId, options) =>
                  runtime.session.navigateTree(targetId, options),
                switchSession: (path, options) =>
                  runtime.switchSession(path, options),
                reload: () => runtime.session.reload(),
              },
            }),
          )
        }
      })

      runtime.setRebindSession(() => Effect.runPromise(bindCurrent))
      yield* bindCurrent

      return {
        session: {
          subscribe: (listener: (event: AgentSessionEvent) => void) => {
            listeners.add(listener)
            return () => listeners.delete(listener)
          },
          dispose: () => undefined,
          getActiveToolNames: () =>
            runtime.session.getActiveToolNames(),
          getSessionStats: () => runtime.session.getSessionStats(),
          prompt: (text: string) => runtime.session.prompt(text),
          compact: (instructions?: string) =>
            runtime.session.compact(instructions),
          abort: () => runtime.session.abort(),
          bindExtensions: ({ uiContext, onError }) =>
            Effect.runPromise(
              Effect.sync(() => {
                bindings = { uiContext, onError }
              }).pipe(Effect.zipRight(bindCurrent)),
            ),
        },
        extensionsResult: runtime.session.resourceLoader.getExtensions(),
        getCommands: () =>
          runtime.session.resourceLoader
            .getExtensions()
            .runtime.getCommands(),
        getMessages: () => runtime.session.messages,
        listSessions: () =>
          Effect.runPromise(
            Effect.tryPromise(() =>
              SessionManager.list(runtime.cwd),
            ).pipe(
              Effect.map((sessions) =>
                sessions.filter(
                  (session) =>
                    session.path !== runtime.session.sessionFile,
                ),
              ),
            ),
          ),
        newSession: () =>
          Effect.runPromise(
            Effect.tryPromise(() => runtime.newSession()).pipe(
              Effect.flatMap((result) =>
                result.cancelled
                  ? Effect.fail(
                      new Error(
                        "New session was cancelled by an extension",
                      ),
                    )
                  : Effect.sync(() => runtime.session.messages),
              ),
            ),
          ),
        resume: (path: string) =>
          Effect.runPromise(
            Effect.tryPromise(() =>
              runtime.switchSession(path),
            ).pipe(
              Effect.flatMap((result) =>
                result.cancelled
                  ? Effect.fail(
                      new Error(
                        "Session resume was cancelled by an extension",
                      ),
                    )
                  : Effect.sync(() => runtime.session.messages),
              ),
            ),
          ),
        shutdown: () =>
          Effect.runPromise(
            Effect.sync(() => unsubscribe()).pipe(
              Effect.zipRight(
                Effect.tryPromise(() => runtime.dispose()),
              ),
            ),
          ),
      }
    }),
  )
}

const sdkCall = (
  operation: "prompt" | "compact" | "abort",
  call: () => Promise<unknown>,
): Effect.Effect<void, PiSessionError> =>
  Effect.tryPromise({
    try: call,
    catch: (cause) => new PiSessionError({ operation, cause }),
  }).pipe(Effect.asVoid)

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
          try: () => open(config.cwd, config.saveSessions),
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
        commands: Effect.sync(result.getCommands),
        sessionStats: Effect.sync(() => result.session.getSessionStats()),
        sessions: Effect.tryPromise({
          try: result.listSessions,
          catch: (cause) =>
            new PiSessionError({ operation: "list", cause }),
        }),
        messages: Effect.sync(result.getMessages),
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
        compact: (instructions) =>
          sdkCall("compact", () =>
            result.session.compact(instructions),
          ),
        newSession: Effect.tryPromise({
          try: result.newSession,
          catch: (cause) =>
            new PiSessionError({ operation: "new", cause }),
        }),
        resume: (path) =>
          Effect.tryPromise({
            try: () => result.resume(path),
            catch: (cause) =>
              new PiSessionError({ operation: "resume", cause }),
          }),
        abort: sdkCall("abort", () => result.session.abort()),
      }
    }),
  )
