import {
  createCliRenderer,
  type CliRenderer,
} from "@opentui/core"
import { Context, Data, Effect, Layer } from "effect"

export interface UiRendererShape {
  readonly renderer: CliRenderer
}

export class UiRenderer extends Context.Tag("pi-opentui/UiRenderer")<
  UiRenderer,
  UiRendererShape
>() {}

export class UiRendererError extends Data.TaggedError("UiRendererError")<{
  readonly cause: unknown
}> {}

export const UiRendererLive = Layer.scoped(
  UiRenderer,
  Effect.acquireRelease(
    Effect.tryPromise({
      try: () => createCliRenderer(),
      catch: (cause) => new UiRendererError({ cause }),
    }).pipe(Effect.map((renderer) => ({ renderer }))),
    ({ renderer }) => Effect.sync(() => renderer.destroy()),
  ),
)
