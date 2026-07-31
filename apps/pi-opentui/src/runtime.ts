import { Layer } from "effect"
import { AppConfigLive } from "./services/app-config.ts"
import { AppStateLive } from "./services/app-state.ts"
import { KeybindingsLive } from "./services/keybindings.ts"
import { makePiSessionLayer } from "./services/pi-session.ts"

export const PiSessionLive = makePiSessionLayer().pipe(
  Layer.provide(AppConfigLive),
)

const StateDependencies = Layer.merge(PiSessionLive, KeybindingsLive)

export const AppLive = AppStateLive.pipe(
  Layer.provideMerge(StateDependencies),
)
