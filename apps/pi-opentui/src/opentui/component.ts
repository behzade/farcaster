import type { Renderable } from "@opentui/core"

export interface OpenTuiComponent<Model> {
  readonly root: Renderable
  update(previous: Model | undefined, current: Model): void
  destroy(): void
}
