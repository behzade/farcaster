import { Layer } from "effect"
import { AppConfigLive } from "./services/app-config.ts"
import { AppStateLive } from "./services/app-state.ts"
import { makePiSessionLayer } from "./services/pi-session.ts"

export const PiSessionLive = makePiSessionLayer().pipe(
  Layer.provide(AppConfigLive),
)

const StateLive = AppStateLive.pipe(Layer.provide(PiSessionLive))

export const AppLive = StateLive
