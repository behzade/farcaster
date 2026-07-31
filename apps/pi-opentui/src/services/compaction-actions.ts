import { Effect } from "effect"
import type { PiSessionError, PiSessionShape } from "./pi-session.ts"
import type { PromptQueueActions } from "./prompt-queue-actions.ts"

export interface CompactionActions {
  readonly begin: Effect.Effect<void>
  readonly finish: (willRetry: boolean) => Effect.Effect<boolean>
  readonly isCompacting: Effect.Effect<boolean>
  readonly queuePrompt: (
    text: string,
    delivery: "steer" | "followUp",
  ) => Effect.Effect<boolean>
}

interface CompactionActionOptions {
  readonly pi: PiSessionShape
  readonly promptQueue: PromptQueueActions
  readonly startPrompt: (text: string) => Effect.Effect<void>
  readonly reportError: (error: PiSessionError) => Effect.Effect<void>
}

export const makeCompactionActions = ({
  pi,
  promptQueue,
  startPrompt,
  reportError,
}: CompactionActionOptions): Effect.Effect<CompactionActions> =>
  Effect.gen(function* () {
    const finish = (willRetry: boolean): Effect.Effect<boolean> =>
      Effect.gen(function* () {
        const prompts = yield* promptQueue.finishCompaction
        if (prompts.length === 0) return false

        return yield* Effect.gen(function* () {
          if (willRetry) {
            for (const prompt of prompts) {
              yield* pi.queuePrompt(prompt.text, prompt.delivery)
            }
            return true
          }

          const [first, ...rest] = prompts
          if (first === undefined) return false
          yield* startPrompt(first.text)
          yield* Effect.yieldNow()
          for (const prompt of rest) {
            yield* pi.queuePrompt(prompt.text, prompt.delivery)
          }
          return true
        }).pipe(
          Effect.catchAll((error) =>
            pi.clearQueue.pipe(
              Effect.catchAll(() =>
                Effect.succeed({ steering: [], followUp: [] }),
              ),
              Effect.zipRight(
                promptQueue.restoreCompactionQueue(prompts),
              ),
              Effect.zipRight(reportError(error)),
              Effect.as(false),
            ),
          ),
        )
      })

    const queuePrompt = (
      text: string,
      delivery: "steer" | "followUp",
    ): Effect.Effect<boolean> =>
      promptQueue.isCompacting.pipe(
        Effect.flatMap((compacting) =>
          compacting
            ? promptQueue
                .queueDuringCompaction(text, delivery)
                .pipe(Effect.as(true))
            : Effect.succeed(false),
        ),
      )

    return {
      begin: promptQueue.beginCompaction,
      finish,
      isCompacting: promptQueue.isCompacting,
      queuePrompt,
    }
  })
