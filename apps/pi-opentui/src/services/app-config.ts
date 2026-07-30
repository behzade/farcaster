import * as Path from "@effect/platform/Path"
import { Context, Effect, Layer } from "effect"

export interface AppConfigShape {
  readonly cwd: string
}

export class AppConfig extends Context.Tag("pi-opentui/AppConfig")<
  AppConfig,
  AppConfigShape
>() {}

export const AppConfigLive = Layer.effect(
  AppConfig,
  Effect.gen(function* () {
    const path = yield* Path.Path

    return {
      cwd: path.resolve(process.cwd()),
    }
  }),
)
