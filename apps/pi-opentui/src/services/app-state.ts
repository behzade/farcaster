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
  type ExtensionLoadError,
  type PiSessionError,
} from "./pi-session.ts"
import {
  appendTranscriptError,
  appendTranscriptNotice,
  appendUserPrompt,
  emptyTranscript,
  reduceTranscriptEvent,
  type TranscriptModel,
} from "./transcript.ts"

export type AppPhase = "ready" | "running" | "stopping" | "error"

export type { AppDialog } from "./extension-ui.ts"
export type { CommandInfo } from "./commands.ts"

export interface AppSnapshot {
  readonly cwd: string
  readonly phase: AppPhase
  readonly activeTools: ReadonlyArray<string>
  readonly extensionPaths: ReadonlyArray<string>
  readonly extensionErrors: ReadonlyArray<ExtensionLoadError>
  readonly eventCount: number
  readonly lastEvent: string | undefined
  readonly error: string | undefined
  readonly transcript: TranscriptModel
  readonly dialog: AppDialog | undefined
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
        extensionPaths: pi.extensionPaths,
        extensionErrors: pi.extensionErrors,
        eventCount: 0,
        lastEvent: undefined,
        error: undefined,
        transcript: initialTranscript,
        dialog: undefined,
        statuses: {},
        commands: builtinCommands,
      })
      const commands = yield* Queue.unbounded<AppCommand>()
      const scope = yield* Effect.scope
      const promptWasAborted = yield* Ref.make(false)

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
        SubscriptionRef.update(state, (snapshot) => ({
          ...snapshot,
          eventCount: snapshot.eventCount + 1,
          lastEvent: event.type,
          transcript: reduceTranscriptEvent(snapshot.transcript, event),
        })),
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
        pi.compact(instructions).pipe(
          Effect.zipRight(
            SubscriptionRef.update(state, (snapshot) => ({
              ...snapshot,
              phase: "ready" as const,
            })),
          ),
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
          default:
            return Effect.succeed(false)
        }
      }

      const abort = Effect.gen(function* () {
        yield* extensionUi.cancelDialog
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
