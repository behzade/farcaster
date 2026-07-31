import { Effect, Ref } from "effect"
import type {
  PushAppNotice,
  UpdateAppState,
} from "./app-state-model.ts"
import type {
  PiSessionError,
  PiSessionShape,
} from "./pi-session.ts"

export interface PromptQueueActions {
  readonly restore: (
    notifyWhenEmpty?: boolean,
  ) => Effect.Effect<void, PiSessionError>
  readonly restoreText: (text: string) => Effect.Effect<void>
}

export const makePromptQueueActions = (
  pi: PiSessionShape,
  updateState: UpdateAppState,
  pushNotice: PushAppNotice,
): Effect.Effect<PromptQueueActions> =>
  Effect.gen(function* () {
    const nextDraftRestoreId = yield* Ref.make(1)

    const putBack = (
      text: string,
      queueWasCleared: boolean,
    ): Effect.Effect<void> =>
      Effect.gen(function* () {
        const id = yield* Ref.getAndUpdate(
          nextDraftRestoreId,
          (current) => current + 1,
        )
        yield* updateState((snapshot) => ({
          ...snapshot,
          promptQueue: queueWasCleared
            ? { steering: [], followUp: [] }
            : snapshot.promptQueue,
          draftRestore: { id, text },
        }))
      })

    const restoreText = (text: string): Effect.Effect<void> =>
      putBack(text, false)

    const restore = (
      notifyWhenEmpty = true,
    ): Effect.Effect<void, PiSessionError> =>
      Effect.gen(function* () {
        const queued = yield* pi.clearQueue
        const text = [...queued.steering, ...queued.followUp]
          .join("\n\n")
          .trim()

        if (text.length === 0) {
          yield* updateState((snapshot) => ({
            ...snapshot,
            promptQueue: { steering: [], followUp: [] },
          }))
          if (notifyWhenEmpty) {
            yield* pushNotice("No queued messages to restore")
          }
          return
        }

        yield* putBack(text, true)
      })

    return { restore, restoreText }
  })
