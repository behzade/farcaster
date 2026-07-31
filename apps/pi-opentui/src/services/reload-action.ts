import { Effect } from "effect"
import { reduceActivity } from "./app-activity.ts"
import { commandCatalog } from "./commands.ts"
import type { KeybindingsShape } from "./keybindings.ts"
import type {
  PiSessionError,
  PiSessionShape,
} from "./pi-session.ts"
import type {
  PushAppNotice,
  UpdateAppState,
} from "./app-state-model.ts"

interface ReloadActionOptions {
  readonly pi: PiSessionShape
  readonly keybindings: KeybindingsShape
  readonly updateState: UpdateAppState
  readonly pushNotice: PushAppNotice
}

export const makeReloadAction = ({
  pi,
  keybindings,
  updateState,
  pushNotice,
}: ReloadActionOptions): Effect.Effect<void, PiSessionError> =>
  Effect.gen(function* () {
    yield* updateState((snapshot) => ({
      ...snapshot,
      activity: reduceActivity(snapshot.activity, {
        _tag: "StartCommand",
        command: "reload",
      }),
    }))
    const reloaded = yield* pi.reload
    yield* Effect.sync(keybindings.reload)
    yield* updateState((snapshot) => ({
      ...snapshot,
      hideThinkingBlock: reloaded.hideThinkingBlock,
      activeTools: reloaded.activeTools,
      extensionPaths: reloaded.extensionPaths,
      extensionErrors: reloaded.extensionErrors,
      commands: commandCatalog(reloaded.commands),
      model: reloaded.modelState.selected,
      thinkingLevel: reloaded.modelState.thinkingLevel,
      activity: reduceActivity(snapshot.activity, {
        _tag: "FinishCommand",
        command: "reload",
      }),
    }))
    yield* pushNotice("Reloaded Pi resources and keybindings")
  })
