import { Effect } from "effect"
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
  const replace = (
    replacement: Effect.Effect<ReadonlyArray<unknown>, PiSessionError>,
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
        liveUsage: emptyLiveUsage,
        transcript: appendTranscriptNotice(
          transcriptFromMessages(messages),
          notice,
        ),
        commands: commandCatalog(sdkCommands),
      }))
    }).pipe(Effect.catchAll(reportError))

  const resume = Effect.gen(function* () {
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
    const session = sessions[choices.indexOf(selected)]
    if (session === undefined) return
    yield* replace(
      pi.resume(session.path),
      `Resumed ${session.name ?? session.id}`,
    )
  }).pipe(Effect.catchAll(reportError))

  return { replace, resume }
}

