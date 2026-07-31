import { Effect, Ref } from "effect"
import type {
  PushAppNotice,
  UpdateAppState,
} from "./app-state-model.ts"
import type {
  PiSessionError,
  PiSessionShape,
  PromptDelivery,
  PromptQueue,
} from "./pi-session.ts"

export interface QueuedCompactionPrompt {
  readonly text: string
  readonly delivery: PromptDelivery
}

export interface PromptQueueActions {
  readonly restore: (
    notifyWhenEmpty?: boolean,
  ) => Effect.Effect<void, PiSessionError>
  readonly restoreText: (text: string) => Effect.Effect<void>
  readonly isCompacting: Effect.Effect<boolean>
  readonly beginCompaction: Effect.Effect<void>
  readonly queueDuringCompaction: (
    text: string,
    delivery: PromptDelivery,
  ) => Effect.Effect<void>
  readonly finishCompaction: Effect.Effect<
    ReadonlyArray<QueuedCompactionPrompt>
  >
  readonly restoreCompactionQueue: (
    prompts: ReadonlyArray<QueuedCompactionPrompt>,
  ) => Effect.Effect<void>
  readonly updateSessionQueue: (queue: PromptQueue) => Effect.Effect<void>
}

export const makePromptQueueActions = (
  pi: PiSessionShape,
  updateState: UpdateAppState,
  pushNotice: PushAppNotice,
): Effect.Effect<PromptQueueActions> =>
  Effect.gen(function* () {
    const nextDraftRestoreId = yield* Ref.make(1)
    const sessionQueue = yield* Ref.make<PromptQueue>({
      steering: [],
      followUp: [],
    })
    const compactionQueue = yield* Ref.make<
      ReadonlyArray<QueuedCompactionPrompt>
    >([])
    const compacting = yield* Ref.make(false)

    const publishQueue = Effect.gen(function* () {
      const session = yield* Ref.get(sessionQueue)
      const local = yield* Ref.get(compactionQueue)
      yield* updateState((snapshot) => ({
        ...snapshot,
        promptQueue: {
          steering: [
            ...session.steering,
            ...local
              .filter((prompt) => prompt.delivery === "steer")
              .map((prompt) => prompt.text),
          ],
          followUp: [
            ...session.followUp,
            ...local
              .filter((prompt) => prompt.delivery === "followUp")
              .map((prompt) => prompt.text),
          ],
        },
      }))
    })

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

    const combinedQueueText = (
      session: PromptQueue,
      local: ReadonlyArray<QueuedCompactionPrompt>,
    ): string =>
      [
        ...session.steering,
        ...local
          .filter((prompt) => prompt.delivery === "steer")
          .map((prompt) => prompt.text),
        ...session.followUp,
        ...local
          .filter((prompt) => prompt.delivery === "followUp")
          .map((prompt) => prompt.text),
      ]
        .join("\n\n")
        .trim()

    const restore = (
      notifyWhenEmpty = true,
    ): Effect.Effect<void, PiSessionError> =>
      Effect.gen(function* () {
        const local = yield* Ref.getAndSet(compactionQueue, [])
        const queued = yield* pi.clearQueue.pipe(
          Effect.catchAll((error) => {
            const localText = combinedQueueText(
              { steering: [], followUp: [] },
              local,
            )
            return publishQueue.pipe(
              Effect.zipRight(
                localText.length > 0
                  ? putBack(localText, false)
                  : Effect.void,
              ),
              Effect.zipRight(Effect.fail(error)),
            )
          }),
        )
        yield* Ref.set(sessionQueue, {
          steering: [],
          followUp: [],
        })
        const text = combinedQueueText(queued, local)

        if (text.length === 0) {
          yield* publishQueue
          if (notifyWhenEmpty) {
            yield* pushNotice("No queued messages to restore")
          }
          return
        }

        yield* putBack(text, true)
      })

    const queueDuringCompaction = (
      text: string,
      delivery: PromptDelivery,
    ): Effect.Effect<void> =>
      Ref.update(compactionQueue, (queued) => [
        ...queued,
        { text, delivery },
      ]).pipe(
        Effect.zipRight(publishQueue),
        Effect.zipRight(pushNotice("Queued message for after compaction")),
      )

    const finishCompaction = Ref.getAndSet(
      compactionQueue,
      [],
    ).pipe(
      Effect.tap(() => Ref.set(compacting, false)),
      Effect.tap(() => publishQueue),
    )

    const restoreCompactionQueue = (
      prompts: ReadonlyArray<QueuedCompactionPrompt>,
    ): Effect.Effect<void> =>
      Ref.update(compactionQueue, (queued) => [
        ...prompts,
        ...queued,
      ]).pipe(Effect.zipRight(publishQueue))

    const updateSessionQueue = (queue: PromptQueue): Effect.Effect<void> =>
      Ref.set(sessionQueue, queue).pipe(Effect.zipRight(publishQueue))

    return {
      restore,
      restoreText,
      isCompacting: Ref.get(compacting),
      beginCompaction: Ref.set(compacting, true),
      queueDuringCompaction,
      finishCompaction,
      restoreCompactionQueue,
      updateSessionQueue,
    }
  })
