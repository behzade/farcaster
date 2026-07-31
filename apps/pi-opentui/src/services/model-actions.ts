import { Effect } from "effect"
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
      transcript: appendTranscriptNotice(snapshot.transcript, notice),
    }))

  const chooseModel = (query: string): Effect.Effect<void> =>
    Effect.gen(function* () {
      yield* updateState((snapshot) => ({
        ...snapshot,
        phase: "running" as const,
        error: undefined,
      }))
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

      const modelState = yield* pi.selectModel(chosen.provider, chosen.id)
      yield* applyModelState(
        modelState,
        `Model: ${chosen.provider}/${chosen.id}`,
      )
    }).pipe(Effect.catchAll(reportError))

  const chooseThinking = (requested: string): Effect.Effect<void> =>
    Effect.gen(function* () {
      yield* updateState((snapshot) => ({
        ...snapshot,
        phase: "running" as const,
        error: undefined,
      }))
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
        yield* pushNotice("Current model does not support thinking", true)
        return
      }

      let level = current.thinkingLevels.find(
        (candidate) => candidate === requested,
      )
      if (requested.length > 0 && level === undefined) {
        yield* updateState((snapshot) => ({
          ...snapshot,
          phase: "ready" as const,
        }))
        yield* pushNotice(`Unknown thinking level: ${requested}`, true)
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
          extensionUi.context.select("Choose thinking level", choices),
        )
        if (selected === undefined) return
        level = current.thinkingLevels[choices.indexOf(selected)]
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
    }).pipe(Effect.catchAll(reportError))

  return { chooseModel, chooseThinking }
}
