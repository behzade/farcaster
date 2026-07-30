import { BunContext } from "@effect/platform-bun"
import { Layer } from "effect"
import { AppConfigLive } from "./services/app-config.ts"
import { AppStateLive } from "./services/app-state.ts"
import { makePiSessionLayer } from "./services/pi-session.ts"
import { UiRendererLive } from "./services/ui-renderer.ts"

export const PiSessionLive = makePiSessionLayer().pipe(
  Layer.provide(AppConfigLive),
)

const StateLive = AppStateLive.pipe(Layer.provide(PiSessionLive))

export const AppLive = Layer.merge(StateLive, UiRendererLive).pipe(
  Layer.provide(BunContext.layer),
)
