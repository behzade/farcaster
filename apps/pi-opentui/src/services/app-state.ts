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
  PromptDelivery,
} from "./app-state-model.ts"
import { applyAppStateUpdate } from "./app-state-model.ts"
import {
  builtinCommands,
  commandCatalog,
  commandHelp,
  sessionStatsText,
  type BuiltinCommandName,
} from "./commands.ts"
import { makeExtensionUi } from "./extension-ui.ts"
import { Keybindings } from "./keybindings.ts"
import { PiSession, PiSessionError } from "./pi-session.ts"
import { runLoginFlow } from "./login-flow.ts"
import { assertAgentSessionEventContract } from "./event-contract.ts"
import { makeModelActions } from "./model-actions.ts"
import { makeReloadAction } from "./reload-action.ts"
import { makeSessionActions } from "./session-actions.ts"
import { makePromptQueueActions } from "./prompt-queue-actions.ts"
import { makeCompactionActions } from "./compaction-actions.ts"
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

const eventRefreshesContextUsage = (eventType: string): boolean =>
  eventType === "message_end" ||
  eventType === "turn_end" ||
  eventType === "entry_appended" ||
  eventType === "compaction_end" ||
  eventType === "agent_settled"

const replaceContextUsage = (
  current: AppSnapshot["sessionStats"],
  latest: AppSnapshot["sessionStats"],
): AppSnapshot["sessionStats"] => {
  const { contextUsage: _oldContextUsage, ...withoutContextUsage } = current
  return latest.contextUsage === undefined
    ? withoutContextUsage
    : { ...withoutContextUsage, contextUsage: latest.contextUsage }
}

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

export const AppStateLive: Layer.Layer<
  AppState,
  never,
  PiSession | Keybindings
> =
  Layer.scoped(
    AppState,
    Effect.gen(function* () {
      const pi = yield* PiSession
      const keybindings = yield* Keybindings
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
        promptQueue: { steering: [], followUp: [] },
        draftRestore: undefined,
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

      const reportQueuedPromptError = (
        error: PiSessionError,
      ): Effect.Effect<void> =>
        updateState((snapshot) => ({
          ...snapshot,
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
      const promptQueueActions = yield* makePromptQueueActions(
        pi,
        updateState,
        pushNotice,
      )
      const compactionActions = yield* makeCompactionActions({
        pi,
        promptQueue: promptQueueActions,
        startPrompt: (prompt) =>
          Ref.set(promptWasAborted, false).pipe(
            Effect.zipRight(
              updateState((snapshot) => ({
                ...snapshot,
                phase: "running" as const,
                error: undefined,
              })),
            ),
            Effect.zipRight(Effect.forkIn(runPrompt(prompt), scope)),
            Effect.asVoid,
          ),
        reportError: reportQueuedPromptError,
      })
      const reload = makeReloadAction({
        pi,
        keybindings,
        updateState,
        pushNotice,
      }).pipe(Effect.catchAll(reportError))

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
            eventRefreshesContextUsage(event.type)
              ? yield* pi.sessionStats
              : undefined
          if (event.type === "compaction_start") {
            yield* compactionActions.begin
          }
          if (event.type === "queue_update") {
            yield* promptQueueActions.updateSessionQueue({
              steering: event.steering,
              followUp: event.followUp,
            })
          }
          const resumedAfterCompaction =
            event.type === "compaction_end"
              ? yield* compactionActions.finish(event.willRetry)
              : false
          yield* updateState((snapshot) => ({
            ...snapshot,
            phase:
              event.type === "agent_start"
                ? "running"
                : event.type === "agent_settled"
                  ? "ready"
                  : event.type === "compaction_end" &&
                      event.reason === "manual"
                    ? resumedAfterCompaction
                      ? snapshot.phase
                      : event.errorMessage === undefined
                        ? "ready"
                        : "error"
                  : snapshot.phase,
            eventCount: snapshot.eventCount + 1,
            lastEvent: event.type,
            error:
              event.type === "compaction_end" && event.errorMessage
                ? event.errorMessage
                : snapshot.error,
            sessionStats:
              latestStats === undefined
                ? snapshot.sessionStats
                : event.type === "agent_settled"
                  ? latestStats
                  : replaceContextUsage(snapshot.sessionStats, latestStats),
            liveUsage:
              event.type === "agent_settled"
                ? emptyLiveUsage
                : reduceLiveUsage(snapshot.liveUsage, event),
            thinkingLevel:
              event.type === "thinking_level_changed"
                ? event.level
                : snapshot.thinkingLevel,
            transcript: reduceTranscriptEvent(
              snapshot.transcript,
              event,
              pi.presentExtensionTool,
            ),
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

      function runPrompt(prompt: string): Effect.Effect<void> {
        return Effect.gen(function* () {
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
      }

      const runCompact = (
        instructions: string | undefined,
      ): Effect.Effect<void> =>
        Effect.gen(function* () {
          yield* pi.compact(instructions)
          const sessionStats = yield* pi.sessionStats
          yield* updateState((snapshot) => ({
            ...snapshot,
            sessionStats,
            liveUsage: emptyLiveUsage,
          }))
        }).pipe(
          // AgentSession reports success, cancellation, and failure through
          // compaction_end; the event fold owns the visible phase and error.
          Effect.catchAll(() => Effect.void),
        )

      const login = (providerRef: string): Effect.Effect<void> =>
        Effect.gen(function* () {
          const controller = new AbortController()
          yield* Ref.set(loginController, controller)
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

      type BuiltinHandler = (
        argumentsText: string,
      ) => Effect.Effect<unknown>

      const builtinHandlers: Readonly<
        Record<BuiltinCommandName, BuiltinHandler>
      > = {
        help: () =>
          SubscriptionRef.get(state).pipe(
            Effect.flatMap((snapshot) =>
              pushNotice(commandHelp(snapshot.commands)),
            ),
          ),
        session: () =>
          pi.sessionStats.pipe(
            Effect.flatMap((stats) =>
              pushNotice(sessionStatsText(stats)),
            ),
          ),
        compact: (argumentsText) => {
          const prompt = `/compact${
            argumentsText.length > 0 ? ` ${argumentsText}` : ""
          }`
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
                argumentsText.length > 0
                  ? argumentsText
                  : undefined,
              ),
              scope,
            )
          })
        },
        model: (argumentsText) =>
          Effect.forkIn(
            modelActions.chooseModel(argumentsText),
            scope,
          ),
        login: (argumentsText) =>
          updateState((snapshot) => ({
            ...snapshot,
            phase: "running" as const,
            error: undefined,
          })).pipe(
            Effect.zipRight(
              Effect.forkIn(login(argumentsText), scope),
            ),
          ),
        thinking: (argumentsText) =>
          Effect.forkIn(
            modelActions.chooseThinking(argumentsText),
            scope,
          ),
        new: () =>
          Effect.forkIn(
            sessionActions.replace(
              pi.newSession,
              "Started a new session",
            ),
            scope,
          ),
        resume: () => Effect.forkIn(sessionActions.resume, scope),
        reload: () => Effect.forkIn(reload, scope),
      }

      const runBuiltin = (
        name: string,
        argumentsText: string,
      ): Effect.Effect<boolean> => {
        if (!Object.hasOwn(builtinHandlers, name)) {
          return Effect.succeed(false)
        }
        return builtinHandlers[name as BuiltinCommandName](
          argumentsText,
        ).pipe(Effect.as(true))
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

        const compacting = yield* compactionActions.isCompacting
        yield* promptQueueActions.restore(false).pipe(
          Effect.catchAll(reportQueuedPromptError),
        )
        yield* Ref.set(promptWasAborted, true)
        yield* updateState((current) => ({
          ...current,
          phase: "stopping" as const,
          error: undefined,
        }))
        if (compacting) {
          yield* pi.abortCompaction
          return
        }
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

      const startPrompt = (
        text: string,
        delivery: PromptDelivery,
      ): Effect.Effect<void> =>
        Effect.gen(function* () {
          const prompt = text.trim()
          if (prompt.length === 0) return
          const snapshot = yield* SubscriptionRef.get(state)
          if (
            snapshot.phase === "stopping" ||
            snapshot.phase === "fatal"
          ) {
            return
          }
          if (snapshot.phase === "running") {
            if (yield* compactionActions.queuePrompt(prompt, delivery)) {
              return
            }
            yield* Effect.forkIn(
              pi.prompt(prompt, delivery).pipe(
                Effect.catchAll((error) =>
                  promptQueueActions.restoreText(prompt).pipe(
                    Effect.zipRight(reportQueuedPromptError(error)),
                  ),
                ),
              ),
              scope,
            )
            return
          }
          yield* Ref.set(promptWasAborted, false)
          yield* updateState((current) => ({
            ...current,
            phase: "running" as const,
            error: undefined,
          }))
          yield* Effect.forkIn(runPrompt(prompt), scope)
        })

      const handleCommand = (command: AppCommand): Effect.Effect<void> => {
        switch (command._tag) {
          case "Prompt":
            return startPrompt(command.text, command.delivery)
          case "RunCommand":
            return Effect.gen(function* () {
              const snapshot = yield* SubscriptionRef.get(state)
              if (
                snapshot.phase === "stopping" ||
                snapshot.phase === "fatal"
              ) {
                return
              }
              const selected = snapshot.commands.find(
                (candidate) => candidate.name === command.name,
              )
              if (selected === undefined) {
                yield* pushNotice(
                  `Unknown command: /${command.name}`,
                  true,
                )
                return
              }
              const prompt = `/${selected.name}${
                command.arguments.length > 0
                  ? ` ${command.arguments}`
                  : ""
              }`
              if (
                snapshot.phase !== "running" &&
                (yield* runBuiltin(
                  selected.name,
                  command.arguments,
                ))
              ) {
                return
              }
              if (
                snapshot.phase === "running" &&
                selected.source === "builtin"
              ) {
                yield* pushNotice(
                  `Cannot run /${selected.name} while Pi is working`,
                  true,
                )
                return
              }
              if (
                snapshot.phase === "running" &&
                selected.source === "extension" &&
                (yield* compactionActions.isCompacting)
              ) {
                yield* Effect.forkIn(
                  pi.prompt(prompt).pipe(
                    Effect.catchAll((error) =>
                      promptQueueActions.restoreText(prompt).pipe(
                        Effect.zipRight(reportQueuedPromptError(error)),
                      ),
                    ),
                  ),
                  scope,
                )
                return
              }
              yield* startPrompt(prompt, command.delivery)
            })
          case "Abort":
            return abort
          case "Dequeue":
            return promptQueueActions.restore().pipe(
              Effect.catchAll(reportQueuedPromptError),
            )
          case "AcknowledgeDraftRestore":
            return updateState((snapshot) => ({
              ...snapshot,
              draftRestore:
                snapshot.draftRestore?.id === command.id
                  ? undefined
                  : snapshot.draftRestore,
            }))
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
