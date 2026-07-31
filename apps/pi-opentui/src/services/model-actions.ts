import { Effect } from "effect"
import { reduceActivity } from "./app-activity.ts"
import type {
  PushAppNotice,
  UpdateAppState,
} from "./app-state-model.ts"
import type { ExtensionUiBridge } from "./extension-ui.ts"
import type {
  PiModelInfo,
  PiModelState,
  PiSessionError,
  PiSessionShape,
  PiThinkingLevel,
} from "./pi-session.ts"
import { appendTranscriptNotice } from "./transcript.ts"

export interface ModelActions {
  readonly chooseModel: (query: string) => Effect.Effect<void>
  readonly chooseThinking: (requested: string) => Effect.Effect<void>
}

export interface ModelActionOptions {
  readonly pi: PiSessionShape
  readonly extensionUi: ExtensionUiBridge
  readonly updateState: UpdateAppState
  readonly pushNotice: PushAppNotice
  readonly reportError: (error: PiSessionError) => Effect.Effect<void>
}

const modelLabel = (model: PiModelInfo): string => {
  const name = model.name === model.id ? "" : ` — ${model.name}`
  return `${model.provider}/${model.id}${name}`
}

const thinkingDescriptions: Readonly<Record<PiThinkingLevel, string>> = {
  off: "No reasoning",
  minimal: "Very brief reasoning",
  low: "Light reasoning",
  medium: "Moderate reasoning",
  high: "Deep reasoning",
  xhigh: "Extra-high reasoning",
  max: "Maximum reasoning",
}

export const makeModelActions = ({
  pi,
  extensionUi,
  updateState,
  pushNotice,
  reportError,
}: ModelActionOptions): ModelActions => {
  const transitionCommand = (
    _tag: "StartCommand" | "FinishCommand",
    command: "model" | "thinking",
  ): Effect.Effect<void> =>
    updateState((snapshot) => ({
      ...snapshot,
      activity: reduceActivity(snapshot.activity, { _tag, command }),
    }))

  const applyModelState = (
    modelState: PiModelState,
    notice: string,
    command: "model" | "thinking",
  ): Effect.Effect<void> =>
    updateState((snapshot) => ({
      ...snapshot,
      activity: reduceActivity(snapshot.activity, {
        _tag: "FinishCommand",
        command,
      }),
      model: modelState.selected,
      thinkingLevel: modelState.thinkingLevel,
      transcript: appendTranscriptNotice(snapshot.transcript, notice),
    }))

  const chooseModel = (query: string): Effect.Effect<void> =>
    Effect.gen(function* () {
      yield* transitionCommand("StartCommand", "model")
      const models = yield* pi.models
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

      let chosen = exactMatches.length === 1 ? exactMatches[0] : undefined
      if (chosen === undefined) {
        if (models.length === 0) {
          yield* transitionCommand("FinishCommand", "model")
          yield* pushNotice("No models available", true)
          return
        }

        const choices = models.map(modelLabel)
        const selected = yield* Effect.promise(() =>
          extensionUi.search("Choose model", choices, query),
        )
        if (selected === undefined) {
          yield* transitionCommand("FinishCommand", "model")
          return
        }
        chosen = models[choices.indexOf(selected)]
        if (chosen === undefined) {
          yield* transitionCommand("FinishCommand", "model")
          yield* pushNotice("Selected model is no longer available", true)
          return
        }
      }

      const modelState = yield* pi.selectModel(chosen.provider, chosen.id)
      yield* applyModelState(
        modelState,
        `Model: ${chosen.provider}/${chosen.id}`,
        "model",
      )
    }).pipe(Effect.catchAll(reportError))

  const chooseThinking = (requested: string): Effect.Effect<void> =>
    Effect.gen(function* () {
      yield* transitionCommand("StartCommand", "thinking")
      const current = yield* pi.modelState
      if (current.selected === undefined) {
        yield* transitionCommand("FinishCommand", "thinking")
        yield* pushNotice("No model selected", true)
        return
      }
      if (!current.selected.reasoning) {
        yield* transitionCommand("FinishCommand", "thinking")
        yield* pushNotice("Current model does not support thinking", true)
        return
      }

      let level = current.thinkingLevels.find(
        (candidate) => candidate === requested,
      )
      if (requested.length > 0 && level === undefined) {
        yield* transitionCommand("FinishCommand", "thinking")
        yield* pushNotice(`Unknown thinking level: ${requested}`, true)
        return
      }
      if (level === undefined) {
        const choices = current.thinkingLevels.map(
          (candidate) =>
            `${candidate} — ${thinkingDescriptions[candidate]}`,
        )
        const selected = yield* Effect.promise(() =>
          extensionUi.context.select("Choose thinking level", choices),
        )
        if (selected === undefined) {
          yield* transitionCommand("FinishCommand", "thinking")
          return
        }
        level = current.thinkingLevels[choices.indexOf(selected)]
        if (level === undefined) {
          yield* transitionCommand("FinishCommand", "thinking")
          yield* pushNotice("Selected thinking level is no longer available", true)
          return
        }
      }

      const modelState = yield* pi.selectThinking(level)
      yield* applyModelState(
        modelState,
        `Thinking level: ${modelState.thinkingLevel}`,
        "thinking",
      )
    }).pipe(Effect.catchAll(reportError))

  return { chooseModel, chooseThinking }
}
