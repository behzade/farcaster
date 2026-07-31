import { Effect } from "effect"
import { reduceActivity } from "./app-activity.ts"
import { commandCatalog } from "./commands.ts"
import type { ExtensionUiBridge } from "./extension-ui.ts"
import { emptyLiveUsage } from "./live-usage.ts"
import type {
  PushAppNotice,
  UpdateAppState,
} from "./app-state-model.ts"
import type { PiSessionError, PiSessionShape } from "./pi-session.ts"
import {
  appendTranscriptNotice,
  transcriptFromMessages,
} from "./transcript.ts"

export interface SessionActions {
  readonly replace: (
    replacement: Effect.Effect<ReadonlyArray<unknown>, PiSessionError>,
    notice: string,
    command: "new-session" | "resume",
  ) => Effect.Effect<void>
  readonly resume: Effect.Effect<void>
}

export interface SessionActionOptions {
  readonly pi: PiSessionShape
  readonly extensionUi: ExtensionUiBridge
  readonly updateState: UpdateAppState
  readonly pushNotice: PushAppNotice
  readonly reportError: (error: PiSessionError) => Effect.Effect<void>
}

export const makeSessionActions = ({
  pi,
  extensionUi,
  updateState,
  pushNotice,
  reportError,
}: SessionActionOptions): SessionActions => {
  const transitionCommand = (
    _tag: "StartCommand" | "FinishCommand",
    command: "new-session" | "resume",
  ): Effect.Effect<void> =>
    updateState((snapshot) => ({
      ...snapshot,
      activity: reduceActivity(snapshot.activity, { _tag, command }),
    }))

  const replace = (
    replacement: Effect.Effect<ReadonlyArray<unknown>, PiSessionError>,
    notice: string,
    command: "new-session" | "resume",
  ): Effect.Effect<void> =>
    Effect.gen(function* () {
      yield* transitionCommand("StartCommand", command)
      const messages = yield* replacement
      const sdkCommands = yield* pi.commands
      const modelState = yield* pi.modelState
      const sessionStats = yield* pi.sessionStats
      yield* updateState((snapshot) => ({
        ...snapshot,
        activity: reduceActivity(snapshot.activity, {
          _tag: "FinishCommand",
          command,
        }),
        model: modelState.selected,
        thinkingLevel: modelState.thinkingLevel,
        sessionStats,
        liveUsage: emptyLiveUsage,
        promptQueue: { steering: [], followUp: [] },
        draftRestore: undefined,
        transcript: appendTranscriptNotice(
          transcriptFromMessages(messages, pi.presentExtensionTool),
          notice,
        ),
        commands: commandCatalog(sdkCommands),
      }))
    }).pipe(Effect.catchAll(reportError))

  const resume = Effect.gen(function* () {
    yield* transitionCommand("StartCommand", "resume")
    const sessions = yield* pi.sessions
    if (sessions.length === 0) {
      yield* transitionCommand("FinishCommand", "resume")
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
    if (selected === undefined) {
      yield* transitionCommand("FinishCommand", "resume")
      return
    }
    const session = sessions[choices.indexOf(selected)]
    if (session === undefined) {
      yield* transitionCommand("FinishCommand", "resume")
      yield* pushNotice("Selected session is no longer available", true)
      return
    }
    yield* replace(
      pi.resume(session.path),
      `Resumed ${session.name ?? session.id}`,
      "resume",
    )
  }).pipe(Effect.catchAll(reportError))

  return { replace, resume }
}
