import {
  Context,
  Effect,
  Layer,
  Queue,
  Ref,
  Stream,
  SubscriptionRef,
} from "effect"
import type {
  AppCommand,
  AppPhase,
  AppSnapshot,
} from "./app-state-model.ts"
import { applyAppStateUpdate } from "./app-state-model.ts"
import {
  builtinCommands,
  commandCatalog,
  commandHelp,
  commandName,
  sessionStatsText,
} from "./commands.ts"
import { makeExtensionUi } from "./extension-ui.ts"
import { PiSession, PiSessionError } from "./pi-session.ts"
import { runLoginFlow } from "./login-flow.ts"
import { assertAgentSessionEventContract } from "./event-contract.ts"
import { makeModelActions } from "./model-actions.ts"
import { makeSessionActions } from "./session-actions.ts"
import {
  emptyLiveUsage,
  reduceLiveUsage,
} from "./live-usage.ts"
import {
  appendTranscriptError,
  appendTranscriptNotice,
  appendUserPrompt,
  emptyTranscript,
  reduceTranscriptEvent,
} from "./transcript.ts"

const unlessFatal = (current: AppPhase, next: AppPhase): AppPhase =>
  current === "fatal" ? "fatal" : next

export type { AppDialog } from "./extension-ui.ts"
export type { CommandInfo } from "./commands.ts"
export type {
  AppCommand,
  AppPhase,
  AppSnapshot,
} from "./app-state-model.ts"

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
        hideThinkingBlock: pi.hideThinkingBlock,
        phase: "ready",
        activeTools: pi.activeTools,
        model: initialModelState.selected,
        thinkingLevel: initialModelState.thinkingLevel,
        sessionStats: initialSessionStats,
        liveUsage: emptyLiveUsage,
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
      ): Effect.Effect<void> =>
        SubscriptionRef.update(state, (snapshot) =>
          applyAppStateUpdate(snapshot, update),
        )

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

      const reportError = (error: PiSessionError): Effect.Effect<void> =>
        updateState((snapshot) => ({
          ...snapshot,
          phase: "error" as const,
          error: errorText(error),
          transcript: appendTranscriptError(
            snapshot.transcript,
            errorText(error),
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
      const sessionActions = makeSessionActions({
        pi,
        extensionUi,
        updateState,
        pushNotice,
        reportError,
      })
      const modelActions = makeModelActions({
        pi,
        extensionUi,
        updateState,
        pushNotice,
        reportError,
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
          yield* Effect.try({
            try: () => assertAgentSessionEventContract(event),
            catch: (cause) =>
              cause instanceof Error ? cause : new Error(String(cause)),
          })
          const latestStats =
            event.type === "agent_settled"
              ? yield* pi.sessionStats
              : undefined
          yield* updateState((snapshot) => ({
            ...snapshot,
            phase:
              event.type === "agent_start"
                ? "running"
                : event.type === "agent_settled"
                  ? "ready"
                  : snapshot.phase,
            eventCount: snapshot.eventCount + 1,
            lastEvent: event.type,
            sessionStats: latestStats ?? snapshot.sessionStats,
            liveUsage:
              event.type === "agent_settled"
                ? emptyLiveUsage
                : reduceLiveUsage(snapshot.liveUsage, event),
            thinkingLevel:
              event.type === "thinking_level_changed"
                ? event.level
                : snapshot.thinkingLevel,
            transcript: reduceTranscriptEvent(snapshot.transcript, event),
          }))
        }),
      ).pipe(
        Effect.catchAll((error) =>
          updateState((snapshot) => ({
            ...snapshot,
            phase: "fatal" as const,
            error: error.message,
            transcript: appendTranscriptError(
              snapshot.transcript,
              error.message,
            ),
          })),
        ),
        Effect.forkScoped,
      )

      const runPrompt = (prompt: string): Effect.Effect<void> =>
        Effect.gen(function* () {
          yield* pi.prompt(prompt)
          yield* updateState((snapshot) => ({
            ...snapshot,
            phase: unlessFatal(snapshot.phase, "ready"),
          }))
        }).pipe(
          Effect.catchAll((error) =>
            Effect.gen(function* () {
              const wasAborted = yield* Ref.get(promptWasAborted)
              yield* updateState((snapshot) =>
                wasAborted
                  ? {
                      ...snapshot,
                      phase: unlessFatal(snapshot.phase, "ready"),
                      error: undefined,
                    }
                  : {
                      ...snapshot,
                      phase: unlessFatal(snapshot.phase, "error"),
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
          yield* updateState((snapshot) => ({
            ...snapshot,
            phase: "ready" as const,
            sessionStats,
            liveUsage: emptyLiveUsage,
          }))
        }).pipe(
          Effect.catchAll((error) =>
            Effect.gen(function* () {
              const wasAborted = yield* Ref.get(promptWasAborted)
              yield* updateState((snapshot) =>
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
              yield* updateState((snapshot) => ({
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
            return Effect.forkIn(
              modelActions.chooseModel(prompt),
              scope,
            ).pipe(
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
            return Effect.forkIn(
              modelActions.chooseThinking(prompt),
              scope,
            ).pipe(
              Effect.as(true),
            )
          case "new":
            return Effect.forkIn(
              sessionActions.replace(
                pi.newSession,
                "Started a new session",
              ),
              scope,
            ).pipe(
              Effect.as(true),
            )
          case "resume":
            return Effect.forkIn(sessionActions.resume, scope).pipe(
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
        yield* updateState((current) => ({
          ...current,
          phase: "stopping" as const,
          error: undefined,
        }))
        yield* pi.abort
        yield* updateState((current) => ({
          ...current,
          phase: "ready" as const,
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

      const handleCommand = (command: AppCommand): Effect.Effect<void> => {
        switch (command._tag) {
          case "Prompt":
            return Effect.gen(function* () {
              const prompt = command.text.trim()
              if (prompt.length === 0) return

              const snapshot = yield* SubscriptionRef.get(state)
              if (
                snapshot.phase === "running" ||
                snapshot.phase === "stopping" ||
                snapshot.phase === "fatal"
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
              yield* updateState((current) => ({
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
