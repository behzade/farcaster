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
import {
  piAuthProviders,
  type PiAuthInteraction,
  type PiAuthProvider,
  type PiAuthType,
} from "./pi-auth.ts"

export type PromptDelivery = "steer" | "followUp"

export interface PromptQueue {
  readonly steering: ReadonlyArray<string>
  readonly followUp: ReadonlyArray<string>
}

export interface ExtensionLoadError {
  readonly path: string
  readonly error: string
}

export type PiThinkingLevel =
  | "off"
  | "minimal"
  | "low"
  | "medium"
  | "high"
  | "xhigh"
  | "max"

export interface PiModelInfo {
  readonly provider: string
  readonly id: string
  readonly name: string
  readonly reasoning: boolean
}

export interface PiModelState {
  readonly selected: PiModelInfo | undefined
  readonly thinkingLevel: PiThinkingLevel
  readonly thinkingLevels: ReadonlyArray<PiThinkingLevel>
}

export interface PiReloadState {
  readonly hideThinkingBlock: boolean
  readonly activeTools: ReadonlyArray<string>
  readonly extensionPaths: ReadonlyArray<string>
  readonly extensionErrors: ReadonlyArray<ExtensionLoadError>
  readonly commands: ReadonlyArray<SlashCommandInfo>
  readonly modelState: PiModelState
}

export interface PiSessionShape {
  readonly cwd: string
  readonly hideThinkingBlock: boolean
  readonly activeTools: ReadonlyArray<string>
  readonly extensionPaths: ReadonlyArray<string>
  readonly extensionErrors: ReadonlyArray<ExtensionLoadError>
  readonly events: Stream.Stream<AgentSessionEvent>
  readonly commands: Effect.Effect<ReadonlyArray<SlashCommandInfo>>
  readonly sessionStats: Effect.Effect<SessionStats>
  readonly modelState: Effect.Effect<PiModelState>
  readonly models: Effect.Effect<ReadonlyArray<PiModelInfo>, PiSessionError>
  readonly authProviders: Effect.Effect<
    ReadonlyArray<PiAuthProvider>,
    PiSessionError
  >
  readonly sessions: Effect.Effect<ReadonlyArray<SessionInfo>, PiSessionError>
  readonly messages: Effect.Effect<ReadonlyArray<unknown>>
  readonly bindExtensions: (
    uiContext: ExtensionUIContext,
    onError: (error: ExtensionError) => void,
  ) => Effect.Effect<void, PiSessionError>
  readonly prompt: (
    text: string,
    delivery?: PromptDelivery,
  ) => Effect.Effect<void, PiSessionError>
  readonly clearQueue: Effect.Effect<PromptQueue, PiSessionError>
  readonly compact: (
    instructions?: string,
  ) => Effect.Effect<void, PiSessionError>
  readonly reload: Effect.Effect<PiReloadState, PiSessionError>
  readonly newSession: Effect.Effect<ReadonlyArray<unknown>, PiSessionError>
  readonly resume: (
    path: string,
  ) => Effect.Effect<ReadonlyArray<unknown>, PiSessionError>
  readonly selectModel: (
    provider: string,
    id: string,
  ) => Effect.Effect<PiModelState, PiSessionError>
  readonly selectThinking: (
    level: PiThinkingLevel,
  ) => Effect.Effect<PiModelState, PiSessionError>
  readonly login: (
    provider: string,
    type: PiAuthType,
    interaction: PiAuthInteraction,
  ) => Effect.Effect<void, PiSessionError>
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
    | "queue"
    | "compact"
    | "reload"
    | "models"
    | "auth"
    | "login"
    | "model"
    | "thinking"
    | "list"
    | "new"
    | "resume"
    | "abort"
    | "shutdown"
  readonly cause: unknown
}> {}

export interface OpenedPiSession {
  readonly shutdown: () => Promise<void>
  readonly getHideThinkingBlock: () => boolean
  readonly getCommands: () => Array<SlashCommandInfo>
  readonly getModelState: () => PiModelState
  readonly listModels: () => Promise<Array<PiModelInfo>>
  readonly listAuthProviders: () => Promise<Array<PiAuthProvider>>
  readonly listSessions: () => Promise<Array<SessionInfo>>
  readonly getMessages: () => ReadonlyArray<unknown>
  readonly newSession: () => Promise<ReadonlyArray<unknown>>
  readonly resume: (path: string) => Promise<ReadonlyArray<unknown>>
  readonly selectModel: (
    provider: string,
    id: string,
  ) => Promise<PiModelState>
  readonly selectThinking: (
    level: PiThinkingLevel,
  ) => Promise<PiModelState>
  readonly login: (
    provider: string,
    type: PiAuthType,
    interaction: PiAuthInteraction,
  ) => Promise<void>
  readonly reload: () => Promise<PiReloadState>
  readonly session: {
    readonly subscribe: (
      listener: (event: AgentSessionEvent) => void,
    ) => () => void
    readonly dispose: () => void
    readonly getActiveToolNames: () => Array<string>
    readonly getSessionStats: () => SessionStats
    readonly prompt: (
      text: string,
      delivery?: PromptDelivery,
    ) => Promise<void>
    readonly clearQueue: () => PromptQueue
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

      const toModelInfo = (model: {
        readonly provider: string
        readonly id: string
        readonly name: string
        readonly reasoning: boolean
      }): PiModelInfo => ({
        provider: model.provider,
        id: model.id,
        name: model.name,
        reasoning: model.reasoning,
      })

      const getModelState = (): PiModelState => ({
        selected:
          runtime.session.model === undefined
            ? undefined
            : toModelInfo(runtime.session.model),
        thinkingLevel: runtime.session.thinkingLevel,
        thinkingLevels:
          runtime.session.getAvailableThinkingLevels(),
      })

      const modelInfos = (
        models: ReadonlyArray<{
          readonly provider: string
          readonly id: string
          readonly name: string
          readonly reasoning: boolean
        }>,
      ): Array<PiModelInfo> => {
        const current = runtime.session.model
        return models.map(toModelInfo).toSorted((left, right) => {
          const leftIsCurrent =
            left.provider === current?.provider &&
            left.id === current?.id
          const rightIsCurrent =
            right.provider === current?.provider &&
            right.id === current?.id
          if (leftIsCurrent !== rightIsCurrent) {
            return leftIsCurrent ? -1 : 1
          }
          return `${left.provider}/${left.id}`.localeCompare(
            `${right.provider}/${right.id}`,
          )
        })
      }

      const listAuthProviders = (): Promise<Array<PiAuthProvider>> =>
        Effect.runPromise(
          Effect.sync(() =>
            piAuthProviders(runtime.session.modelRuntime),
          ).pipe(Effect.map((providers) => [...providers])),
        )

      const getReloadState = (): PiReloadState => {
        const extensions =
          runtime.session.resourceLoader.getExtensions()
        return {
          hideThinkingBlock:
            runtime.session.settingsManager.getHideThinkingBlock(),
          activeTools: runtime.session.getActiveToolNames().toSorted(),
          extensionPaths: extensions.extensions
            .map((extension) => extension.path)
            .toSorted(),
          extensionErrors: extensions.errors,
          commands: extensions.runtime.getCommands(),
          modelState: getModelState(),
        }
      }

      return {
        getHideThinkingBlock: () =>
          runtime.session.settingsManager.getHideThinkingBlock(),
        session: {
          subscribe: (listener: (event: AgentSessionEvent) => void) => {
            listeners.add(listener)
            return () => listeners.delete(listener)
          },
          dispose: () => undefined,
          getActiveToolNames: () =>
            runtime.session.getActiveToolNames(),
          getSessionStats: () => runtime.session.getSessionStats(),
          prompt: (text, delivery) =>
            runtime.session.prompt(
              text,
              delivery === undefined
                ? undefined
                : { streamingBehavior: delivery },
            ),
          clearQueue: () => runtime.session.clearQueue(),
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
        getModelState,
        listModels: () =>
          Effect.runPromise(
            Effect.sync(() =>
              modelInfos(
                runtime.session.scopedModels.length > 0
                  ? runtime.session.scopedModels.map(
                      ({ model }) => model,
                    )
                  : runtime.session.modelRuntime.getAvailableSnapshot(),
              ),
            ),
          ),
        listAuthProviders,
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
        selectModel: (provider: string, id: string) =>
          Effect.runPromise(
            Effect.gen(function* () {
              const model = runtime.session.modelRuntime.getModel(
                provider,
                id,
              )
              if (model === undefined) {
                return yield* Effect.fail(
                  new Error(`Unknown model: ${provider}/${id}`),
                )
              }
              yield* Effect.tryPromise(() =>
                runtime.session.setModel(model),
              )
              return getModelState()
            }),
          ),
        selectThinking: (level: PiThinkingLevel) =>
          Effect.runPromise(
            Effect.sync(() => {
              runtime.session.setThinkingLevel(level)
              return getModelState()
            }),
          ),
        login: (provider, type, interaction) =>
          Effect.runPromise(
            Effect.tryPromise(() =>
              runtime.session.modelRuntime.login(
                provider,
                type,
                interaction,
              ),
            ).pipe(Effect.asVoid),
          ),
        reload: () =>
          Effect.runPromise(
            Effect.tryPromise(() => runtime.session.reload()).pipe(
              Effect.map(getReloadState),
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
        hideThinkingBlock: result.getHideThinkingBlock(),
        activeTools: result.session.getActiveToolNames().toSorted(),
        extensionPaths: result.extensionsResult.extensions
          .map((extension) => extension.path)
          .toSorted(),
        extensionErrors: result.extensionsResult.errors,
        events: sessionEvents(result.session),
        commands: Effect.sync(result.getCommands),
        sessionStats: Effect.sync(() => result.session.getSessionStats()),
        modelState: Effect.sync(result.getModelState),
        models: Effect.tryPromise({
          try: result.listModels,
          catch: (cause) =>
            new PiSessionError({ operation: "models", cause }),
        }),
        authProviders: Effect.tryPromise({
          try: result.listAuthProviders,
          catch: (cause) =>
            new PiSessionError({ operation: "auth", cause }),
        }),
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
        prompt: (text, delivery) =>
          sdkCall("prompt", () => result.session.prompt(text, delivery)),
        clearQueue: Effect.try({
          try: result.session.clearQueue,
          catch: (cause) =>
            new PiSessionError({ operation: "queue", cause }),
        }),
        compact: (instructions) =>
          sdkCall("compact", () =>
            result.session.compact(instructions),
          ),
        reload: Effect.tryPromise({
          try: result.reload,
          catch: (cause) =>
            new PiSessionError({ operation: "reload", cause }),
        }),
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
        selectModel: (provider, id) =>
          Effect.tryPromise({
            try: () => result.selectModel(provider, id),
            catch: (cause) =>
              new PiSessionError({ operation: "model", cause }),
          }),
        selectThinking: (level) =>
          Effect.tryPromise({
            try: () => result.selectThinking(level),
            catch: (cause) =>
              new PiSessionError({ operation: "thinking", cause }),
          }),
        login: (provider, type, interaction) =>
          Effect.tryPromise({
            try: () => result.login(provider, type, interaction),
            catch: (cause) =>
              new PiSessionError({ operation: "login", cause }),
          }),
        abort: sdkCall("abort", () => result.session.abort()),
      }
    }),
  )
