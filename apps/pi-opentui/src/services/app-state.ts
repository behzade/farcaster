import type { SessionStats } from "@earendil-works/pi-coding-agent"
import {
  Context,
  Effect,
  Layer,
  Queue,
  Ref,
  Stream,
  SubscriptionRef,
} from "effect"
import {
  builtinCommands,
  commandCatalog,
  commandHelp,
  commandName,
  sessionStatsText,
  type CommandInfo,
} from "./commands.ts"
import {
  makeExtensionUi,
  type AppDialog,
} from "./extension-ui.ts"
import {
  PiSession,
  PiSessionError,
  type ExtensionLoadError,
  type PiModelInfo,
  type PiModelState,
  type PiThinkingLevel,
} from "./pi-session.ts"
import { runLoginFlow } from "./login-flow.ts"
import {
  appendTranscriptError,
  appendTranscriptNotice,
  appendUserPrompt,
  emptyTranscript,
  reduceTranscriptEvent,
  transcriptFromMessages,
  type TranscriptModel,
} from "./transcript.ts"

export type AppPhase = "ready" | "running" | "stopping" | "error"

export type { AppDialog } from "./extension-ui.ts"
export type { CommandInfo } from "./commands.ts"

export interface AppSnapshot {
  readonly cwd: string
  readonly phase: AppPhase
  readonly activeTools: ReadonlyArray<string>
  readonly model: PiModelInfo | undefined
  readonly thinkingLevel: PiThinkingLevel
  readonly sessionStats: SessionStats
  readonly extensionPaths: ReadonlyArray<string>
  readonly extensionErrors: ReadonlyArray<ExtensionLoadError>
  readonly eventCount: number
  readonly lastEvent: string | undefined
  readonly error: string | undefined
  readonly transcript: TranscriptModel
  readonly dialog: AppDialog | undefined
  readonly authNotice: string | undefined
  readonly statuses: Readonly<Record<string, string>>
  readonly commands: ReadonlyArray<CommandInfo>
}

export type AppCommand =
  | { readonly _tag: "Prompt"; readonly text: string }
  | { readonly _tag: "Abort" }
  | {
      readonly _tag: "ResolveDialog"
      readonly id: number
      readonly value: string | undefined
    }

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
      const initialModelState = yield* pi.modelState
      const initialSessionStats = yield* pi.sessionStats
      const initialTranscript = pi.extensionErrors.reduce(
        (transcript, fault) =>
          appendTranscriptNotice(
            transcript,
            `${fault.path}: ${fault.error}`,
            true,
          ),
        emptyTranscript,
      )
      const state = yield* SubscriptionRef.make<AppSnapshot>({
        cwd: pi.cwd,
        phase: "ready",
        activeTools: pi.activeTools,
        model: initialModelState.selected,
        thinkingLevel: initialModelState.thinkingLevel,
        sessionStats: initialSessionStats,
        extensionPaths: pi.extensionPaths,
        extensionErrors: pi.extensionErrors,
        eventCount: 0,
        lastEvent: undefined,
        error: undefined,
        transcript: initialTranscript,
        dialog: undefined,
        authNotice: undefined,
        statuses: {},
        commands: builtinCommands,
      })
      const commands = yield* Queue.unbounded<AppCommand>()
      const scope = yield* Effect.scope
      const promptWasAborted = yield* Ref.make(false)
      const loginController = yield* Ref.make<
        AbortController | undefined
      >(undefined)

      const updateState = (
        update: (snapshot: AppSnapshot) => AppSnapshot,
      ): Effect.Effect<void> => SubscriptionRef.update(state, update)

      const pushNotice = (
        message: string,
        isError = false,
      ): Effect.Effect<void> =>
        updateState((snapshot) => ({
          ...snapshot,
          transcript: appendTranscriptNotice(
            snapshot.transcript,
            message,
            isError,
          ),
        }))

      const extensionUi = yield* makeExtensionUi({
        setDialog: (dialog) =>
          updateState((snapshot) => ({ ...snapshot, dialog })),
        notify: pushNotice,
        setStatus: (key, text) =>
          updateState((snapshot) => {
            const statuses = { ...snapshot.statuses }
            if (text === undefined) delete statuses[key]
            else statuses[key] = text
            return { ...snapshot, statuses }
          }),
        setAuthNotice: (authNotice) =>
          updateState((snapshot) => ({ ...snapshot, authNotice })),
      })

      yield* pi.bindExtensions(extensionUi.context, (extensionError) => {
        extensionUi.notify(
          `${extensionError.extensionPath}: ${extensionError.error}`,
          true,
        )
      }).pipe(
        Effect.catchAll((error) =>
          updateState((snapshot) => ({
            ...snapshot,
            phase: "error" as const,
            error: errorText(error),
            transcript: appendTranscriptError(
              snapshot.transcript,
              errorText(error),
            ),
          })),
        ),
      )
      const sdkCommands = yield* pi.commands
      yield* updateState((snapshot) => ({
        ...snapshot,
        commands: commandCatalog(sdkCommands),
      }))

      yield* Stream.runForEach(pi.events, (event) =>
        Effect.gen(function* () {
          const latestStats =
            event.type === "agent_settled"
              ? yield* pi.sessionStats
              : undefined
          yield* SubscriptionRef.update(state, (snapshot) => ({
            ...snapshot,
            eventCount: snapshot.eventCount + 1,
            lastEvent: event.type,
            sessionStats: latestStats ?? snapshot.sessionStats,
            transcript: reduceTranscriptEvent(snapshot.transcript, event),
          }))
        }),
      ).pipe(Effect.forkScoped)

      const runPrompt = (prompt: string): Effect.Effect<void> =>
        Effect.gen(function* () {
          yield* pi.prompt(prompt)
          yield* SubscriptionRef.update(state, (snapshot) => ({
            ...snapshot,
            phase: "ready" as const,
          }))
        }).pipe(
          Effect.catchAll((error) =>
            Effect.gen(function* () {
              const wasAborted = yield* Ref.get(promptWasAborted)
              yield* SubscriptionRef.update(state, (snapshot) =>
                wasAborted
                  ? {
                      ...snapshot,
                      phase: "ready" as const,
                      error: undefined,
                    }
                  : {
                      ...snapshot,
                      phase: "error" as const,
                      error: errorText(error),
                      transcript: appendTranscriptError(
                        snapshot.transcript,
                        errorText(error),
                      ),
                    },
              )
            }),
          ),
        )

      const runCompact = (
        instructions: string | undefined,
      ): Effect.Effect<void> =>
        Effect.gen(function* () {
          yield* pi.compact(instructions)
          const sessionStats = yield* pi.sessionStats
          yield* SubscriptionRef.update(state, (snapshot) => ({
            ...snapshot,
            phase: "ready" as const,
            sessionStats,
          }))
        }).pipe(
          Effect.catchAll((error) =>
            Effect.gen(function* () {
              const wasAborted = yield* Ref.get(promptWasAborted)
              yield* SubscriptionRef.update(state, (snapshot) =>
                wasAborted
                  ? {
                      ...snapshot,
                      phase: "ready" as const,
                      error: undefined,
                    }
                  : {
                      ...snapshot,
                      phase: "error" as const,
                      error: errorText(error),
                      transcript: appendTranscriptError(
                        snapshot.transcript,
                        errorText(error),
                      ),
                    },
              )
            }),
          ),
        )

      const replaceSession = (
        replacement: Effect.Effect<
          ReadonlyArray<unknown>,
          PiSessionError
        >,
        notice: string,
      ): Effect.Effect<void> =>
        Effect.gen(function* () {
          yield* updateState((snapshot) => ({
            ...snapshot,
            phase: "running" as const,
            error: undefined,
          }))
          const messages = yield* replacement
          const sdkCommands = yield* pi.commands
          const modelState = yield* pi.modelState
          const sessionStats = yield* pi.sessionStats
          yield* updateState((snapshot) => ({
            ...snapshot,
            phase: "ready" as const,
            error: undefined,
            model: modelState.selected,
            thinkingLevel: modelState.thinkingLevel,
            sessionStats,
            transcript: appendTranscriptNotice(
              transcriptFromMessages(messages),
              notice,
            ),
            commands: commandCatalog(sdkCommands),
          }))
        }).pipe(
          Effect.catchAll((error) =>
            updateState((snapshot) => ({
              ...snapshot,
              phase: "error" as const,
              error: errorText(error),
              transcript: appendTranscriptError(
                snapshot.transcript,
                errorText(error),
              ),
            })),
          ),
        )

      const resumeSession = Effect.gen(function* () {
        const sessions = yield* pi.sessions
        if (sessions.length === 0) {
          yield* pushNotice("No saved sessions found")
          return
        }

        const choices = sessions.map((session) => {
          const firstMessage = session.firstMessage.trim()
          const title =
            session.name ??
            (firstMessage.length > 0 ? firstMessage : session.id)
          return `${title} · ${session.id.slice(0, 8)}`
        })
        const selected = yield* Effect.promise(() =>
          extensionUi.context.select("Resume session", choices),
        )
        if (selected === undefined) return
        const index = choices.indexOf(selected)
        const session = sessions[index]
        if (session === undefined) return
        yield* replaceSession(
          pi.resume(session.path),
          `Resumed ${session.name ?? session.id}`,
        )
      }).pipe(
        Effect.catchAll((error) =>
          updateState((snapshot) => ({
            ...snapshot,
            phase: "error" as const,
            error: errorText(error),
            transcript: appendTranscriptError(
              snapshot.transcript,
              errorText(error),
            ),
          })),
        ),
      )

      const applyModelState = (
        modelState: PiModelState,
        notice: string,
      ): Effect.Effect<void> =>
        updateState((snapshot) => ({
          ...snapshot,
          phase: "ready" as const,
          error: undefined,
          model: modelState.selected,
          thinkingLevel: modelState.thinkingLevel,
          transcript: appendTranscriptNotice(
            snapshot.transcript,
            notice,
          ),
        }))

      const modelLabel = (model: PiModelInfo): string => {
        const name = model.name === model.id ? "" : ` — ${model.name}`
        return `${model.provider}/${model.id}${name}`
      }

      const chooseModel = (prompt: string): Effect.Effect<void> =>
        Effect.gen(function* () {
          const models = yield* pi.models
          const query = prompt.slice("/model".length).trim()
          const normalizedQuery = query.toLowerCase()
          const exactMatches =
            normalizedQuery.length === 0
              ? []
              : models.filter(
                  (model) =>
                    `${model.provider}/${model.id}`.toLowerCase() ===
                      normalizedQuery ||
                    model.id.toLowerCase() === normalizedQuery,
                )

          let chosen =
            exactMatches.length === 1 ? exactMatches[0] : undefined
          if (chosen === undefined) {
            yield* updateState((snapshot) => ({
              ...snapshot,
              phase: "ready" as const,
            }))
            if (models.length === 0) {
              yield* pushNotice("No models available", true)
              return
            }

            const choices = models.map(modelLabel)
            const selected = yield* Effect.promise(() =>
              extensionUi.search("Choose model", choices, query),
            )
            if (selected === undefined) return
            chosen = models[choices.indexOf(selected)]
            if (chosen === undefined) return
            yield* updateState((snapshot) => ({
              ...snapshot,
              phase: "running" as const,
            }))
          }

          const modelState = yield* pi.selectModel(
            chosen.provider,
            chosen.id,
          )
          yield* applyModelState(
            modelState,
            `Model: ${chosen.provider}/${chosen.id}`,
          )
        }).pipe(
          Effect.catchAll((error) =>
            updateState((snapshot) => ({
              ...snapshot,
              phase: "error" as const,
              error: errorText(error),
              transcript: appendTranscriptError(
                snapshot.transcript,
                errorText(error),
              ),
            })),
          ),
        )

      const thinkingDescriptions: Readonly<
        Record<PiThinkingLevel, string>
      > = {
        off: "No reasoning",
        minimal: "Very brief reasoning",
        low: "Light reasoning",
        medium: "Moderate reasoning",
        high: "Deep reasoning",
        xhigh: "Extra-high reasoning",
        max: "Maximum reasoning",
      }

      const chooseThinking = (prompt: string): Effect.Effect<void> =>
        Effect.gen(function* () {
          const current = yield* pi.modelState
          if (current.selected === undefined) {
            yield* updateState((snapshot) => ({
              ...snapshot,
              phase: "ready" as const,
            }))
            yield* pushNotice("No model selected", true)
            return
          }
          if (!current.selected.reasoning) {
            yield* updateState((snapshot) => ({
              ...snapshot,
              phase: "ready" as const,
            }))
            yield* pushNotice(
              "Current model does not support thinking",
              true,
            )
            return
          }

          const requested = prompt.slice("/thinking".length).trim()
          let level = current.thinkingLevels.find(
            (candidate) => candidate === requested,
          )
          if (requested.length > 0 && level === undefined) {
            yield* updateState((snapshot) => ({
              ...snapshot,
              phase: "ready" as const,
            }))
            yield* pushNotice(
              `Unknown thinking level: ${requested}`,
              true,
            )
            return
          }
          if (level === undefined) {
            const choices = current.thinkingLevels.map(
              (candidate) =>
                `${candidate} — ${thinkingDescriptions[candidate]}`,
            )
            yield* updateState((snapshot) => ({
              ...snapshot,
              phase: "ready" as const,
            }))
            const selected = yield* Effect.promise(() =>
              extensionUi.context.select(
                "Choose thinking level",
                choices,
              ),
            )
            if (selected === undefined) return
            level =
              current.thinkingLevels[choices.indexOf(selected)]
            if (level === undefined) return
            yield* updateState((snapshot) => ({
              ...snapshot,
              phase: "running" as const,
            }))
          }

          const modelState = yield* pi.selectThinking(level)
          yield* applyModelState(
            modelState,
            `Thinking level: ${modelState.thinkingLevel}`,
          )
        }).pipe(
          Effect.catchAll((error) =>
            updateState((snapshot) => ({
              ...snapshot,
              phase: "error" as const,
              error: errorText(error),
              transcript: appendTranscriptError(
                snapshot.transcript,
                errorText(error),
              ),
            })),
          ),
        )

      const login = (prompt: string): Effect.Effect<void> =>
        Effect.gen(function* () {
          const controller = new AbortController()
          yield* Ref.set(loginController, controller)
          const providerRef = prompt.slice("/login".length).trim()
          const result = yield* runLoginFlow(
            pi,
            extensionUi,
            providerRef,
            controller.signal,
          )
          const modelState = yield* pi.modelState
          yield* updateState((snapshot) => ({
            ...snapshot,
            phase: "ready" as const,
            error: undefined,
            model: modelState.selected,
            thinkingLevel: modelState.thinkingLevel,
          }))
          if (result !== undefined) {
            yield* pushNotice(
              result.loggedIn && modelState.selected === undefined
                ? `${result.message}. Use /model to select a model.`
                : result.message,
            )
          }
        }).pipe(
          Effect.catchAll((error) =>
            Effect.gen(function* () {
              const controller = yield* Ref.get(loginController)
              const message =
                error instanceof PiSessionError
                  ? errorText(error)
                  : error instanceof Error
                    ? error.message
                    : String(error)
              if (
                controller?.signal.aborted ||
                message.toLowerCase().includes("login cancelled")
              ) {
                yield* updateState((snapshot) => ({
                  ...snapshot,
                  phase: "ready" as const,
                  error: undefined,
                }))
                return
              }
              yield* updateState((snapshot) => ({
                ...snapshot,
                phase: "error" as const,
                error: message,
                transcript: appendTranscriptError(
                  snapshot.transcript,
                  message,
                ),
              }))
            }),
          ),
          Effect.ensuring(Ref.set(loginController, undefined)),
        )

      const runBuiltin = (
        name: string,
        prompt: string,
      ): Effect.Effect<boolean> => {
        switch (name) {
          case "help":
            return SubscriptionRef.get(state).pipe(
              Effect.flatMap((snapshot) =>
                pushNotice(commandHelp(snapshot.commands)),
              ),
              Effect.as(true),
            )
          case "session":
            return pi.sessionStats.pipe(
              Effect.flatMap((stats) =>
                pushNotice(sessionStatsText(stats)),
              ),
              Effect.as(true),
            )
          case "compact": {
            const instructions = prompt.slice("/compact".length).trim()
            return Effect.gen(function* () {
              yield* Ref.set(promptWasAborted, false)
              yield* SubscriptionRef.update(state, (snapshot) => ({
                ...snapshot,
                phase: "running" as const,
                error: undefined,
                transcript: appendUserPrompt(
                  snapshot.transcript,
                  prompt,
                ),
              }))
              yield* Effect.forkIn(
                runCompact(
                  instructions.length > 0 ? instructions : undefined,
                ),
                scope,
              )
              return true
            })
          }
          case "model":
            return updateState((snapshot) => ({
              ...snapshot,
              phase: "running" as const,
              error: undefined,
            })).pipe(
              Effect.zipRight(
                Effect.forkIn(chooseModel(prompt), scope),
              ),
              Effect.as(true),
            )
          case "login":
            return updateState((snapshot) => ({
              ...snapshot,
              phase: "running" as const,
              error: undefined,
            })).pipe(
              Effect.zipRight(Effect.forkIn(login(prompt), scope)),
              Effect.as(true),
            )
          case "thinking":
            return updateState((snapshot) => ({
              ...snapshot,
              phase: "running" as const,
              error: undefined,
            })).pipe(
              Effect.zipRight(
                Effect.forkIn(chooseThinking(prompt), scope),
              ),
              Effect.as(true),
            )
          case "new":
            return updateState((snapshot) => ({
              ...snapshot,
              phase: "running" as const,
              error: undefined,
            })).pipe(
              Effect.zipRight(
                Effect.forkIn(
                  replaceSession(
                    pi.newSession,
                    "Started a new session",
                  ),
                  scope,
                ),
              ),
              Effect.as(true),
            )
          case "resume":
            return Effect.forkIn(resumeSession, scope).pipe(
              Effect.as(true),
            )
          default:
            return Effect.succeed(false)
        }
      }

      const abort = Effect.gen(function* () {
        yield* extensionUi.cancelDialog
        const activeLogin = yield* Ref.get(loginController)
        if (activeLogin !== undefined) {
          yield* Effect.sync(() => activeLogin.abort())
          return
        }
        const snapshot = yield* SubscriptionRef.get(state)
        if (snapshot.phase !== "running") return

        yield* Ref.set(promptWasAborted, true)
        yield* SubscriptionRef.update(state, (current) => ({
          ...current,
          phase: "stopping" as const,
          error: undefined,
        }))
        yield* pi.abort
        yield* SubscriptionRef.update(state, (current) => ({
          ...current,
          phase: "ready" as const,
        }))
      }).pipe(
        Effect.catchAll((error) =>
          SubscriptionRef.update(state, (snapshot) => ({
            ...snapshot,
            phase: "error" as const,
            error: errorText(error),
            transcript: appendTranscriptError(
              snapshot.transcript,
              errorText(error),
            ),
          })),
        ),
      )

      const handleCommand = (command: AppCommand): Effect.Effect<void> => {
        switch (command._tag) {
          case "Prompt":
            return Effect.gen(function* () {
              const prompt = command.text.trim()
              if (prompt.length === 0) return

              const snapshot = yield* SubscriptionRef.get(state)
              if (
                snapshot.phase === "running" ||
                snapshot.phase === "stopping"
              ) {
                return
              }
              const name = commandName(prompt)
              if (name !== undefined) {
                if (yield* runBuiltin(name, prompt)) return
                if (
                  !snapshot.commands.some(
                    (candidate) =>
                      candidate.name === name &&
                      candidate.source !== "builtin",
                  )
                ) {
                  yield* pushNotice(`Unknown command: /${name}`, true)
                  return
                }
              }
              yield* Ref.set(promptWasAborted, false)
              yield* SubscriptionRef.update(state, (current) => ({
                ...current,
                phase: "running" as const,
                error: undefined,
                transcript: appendUserPrompt(
                  current.transcript,
                  prompt,
                ),
              }))
              yield* Effect.forkIn(runPrompt(prompt), scope)
            })
          case "Abort":
            return abort
          case "ResolveDialog":
            return extensionUi.resolveDialog(command.id, command.value)
        }
      }

      yield* Stream.fromQueue(commands).pipe(
        Stream.runForEach(handleCommand),
        Effect.forkScoped,
      )

      return {
        get: SubscriptionRef.get(state),
        changes: state.changes,
        dispatch: (command) =>
          Queue.offer(commands, command).pipe(Effect.asVoid),
      }
    }),
  )
