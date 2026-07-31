import { createTestRenderer } from "@opentui/core/testing"
import { expect, test } from "bun:test"
import { Effect } from "effect"
import { SearchMenuView } from "../src/opentui/search-menu-view.ts"
import {
  filterOptions,
  nearbyOptions,
} from "../src/ui/search.ts"

test("filters without regard to case and keeps eight nearby options", () => {
  expect(
    filterOptions(
      ["OpenAI GPT-5", "Anthropic Claude", "OpenCode Big Pickle"],
      "OPENcode",
    ),
  ).toEqual(["OpenCode Big Pickle"])

  const nearby = nearbyOptions(
    Array.from({ length: 12 }, (_, index) => `model-${index}`),
    7,
  )
  expect(nearby).toHaveLength(8)
  expect(nearby[0]).toEqual({ index: 3, option: "model-3" })
  expect(nearby[7]).toEqual({ index: 10, option: "model-10" })
})

test("searches while its input stays active and selects a result", () => {
  let selected: string | undefined

  return Effect.runPromise(
    Effect.acquireUseRelease(
      Effect.gen(function* () {
        const setup = yield* Effect.tryPromise(() =>
          createTestRenderer({ width: 90, height: 24 })
        )
        const view = new SearchMenuView(setup.renderer, {
          title: "Models",
          message: "Choose a model.",
          options: [
            "openai/gpt-5",
            "opencode-go/minimax-m2.1",
            "opencode-go/glm-4.7",
          ],
          resolve: (value) => {
            selected = value
          },
        })
        setup.renderer.root.add(view.root)
        view.focus()
        return { setup, view }
      }),
      ({ setup }) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() => setup.renderOnce())
          yield* Effect.tryPromise(() => setup.mockInput.typeText("GLM"))
          yield* Effect.tryPromise(() => setup.flush())
          yield* Effect.tryPromise(() => setup.renderOnce())

          const frame = setup.captureCharFrame()
          expect(frame).toContain("GLM")
          expect(frame).toContain("opencode-go/glm-4.7")
          expect(frame).not.toContain("openai/gpt-5")

          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(selected).toBe("opencode-go/glm-4.7")
        }),
      ({ setup, view }) =>
        Effect.sync(() => {
          view.destroy()
          setup.renderer.destroy()
        }),
    ),
  )
})

test("moves through results and cancels when there are no matches", () => {
  const resolutions: Array<string | undefined> = []

  return Effect.runPromise(
    Effect.acquireUseRelease(
      Effect.gen(function* () {
        const setup = yield* Effect.tryPromise(() =>
          createTestRenderer({ width: 80, height: 22 })
        )
        const view = new SearchMenuView(setup.renderer, {
          title: "Commands",
          initialQuery: "zz",
          options: ["/help", "/model", "/resume"],
          resolve: (value) => {
            resolutions.push(value)
          },
        })
        setup.renderer.root.add(view.root)
        view.focus()
        return { setup, view }
      }),
      ({ setup }) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() => setup.renderOnce())
          expect(setup.captureCharFrame()).toContain("No matches")

          setup.mockInput.pressEnter()
          yield* Effect.tryPromise(() => setup.flush())
          expect(resolutions).toEqual([])

          setup.mockInput.pressEscape()
          yield* Effect.sleep("30 millis")
          yield* Effect.tryPromise(() => setup.flush())
          expect(resolutions).toEqual([undefined])
        }),
      ({ setup, view }) =>
        Effect.sync(() => {
          view.destroy()
          setup.renderer.destroy()
        }),
    ),
  )
})
