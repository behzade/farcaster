import { createTestRenderer } from "@opentui/core/testing"
import { expect, test } from "bun:test"
import { Effect } from "effect"
import { TranscriptView } from "../src/opentui/transcript-view.ts"
import type {
  TranscriptModel,
  TranscriptRow,
} from "../src/services/transcript.ts"

const row = (id: string, content: string): TranscriptRow => ({
  id,
  kind: "assistant",
  title: "pi",
  content,
  pending: false,
  isError: false,
})

const model = (rows: ReadonlyArray<TranscriptRow>): TranscriptModel => ({
  rows,
  activeAssistantId: undefined,
  nextRowId: rows.length + 1,
})

test("appends, reorders, and trims owned transcript rows", () =>
  Effect.runPromise(
    Effect.acquireUseRelease(
      Effect.gen(function* () {
        const setup = yield* Effect.tryPromise(() =>
          createTestRenderer({ width: 60, height: 16 })
        )
        const view = new TranscriptView(setup.renderer)
        setup.renderer.root.add(view.root)
        return { setup, view }
      }),
      ({ setup, view }) =>
        Effect.gen(function* () {
          const first = model([row("a", "first"), row("b", "second")])
          view.update(undefined, first)
          yield* Effect.tryPromise(() => setup.renderOnce())
          expect(view.root.getChildrenCount()).toBe(3)

          const reordered = model([
            row("b", "second changed"),
            row("a", "first changed"),
          ])
          view.update(first, reordered)
          yield* Effect.tryPromise(() => setup.flush())
          const reorderedFrame = setup.captureCharFrame()
          expect(reorderedFrame.indexOf("second changed")).toBeLessThan(
            reorderedFrame.indexOf("first changed"),
          )
          expect(view.root.getChildrenCount()).toBe(3)

          const trimmed = model([
            reordered.rows[0]!,
            row("c", "third"),
          ])
          view.update(reordered, trimmed)
          yield* Effect.tryPromise(() => setup.flush())
          const trimmedFrame = setup.captureCharFrame()
          expect(trimmedFrame).toContain("second changed")
          expect(trimmedFrame).toContain("third")
          expect(trimmedFrame).not.toContain("first changed")
          expect(view.root.getChildrenCount()).toBe(3)
        }),
      ({ setup, view }) =>
        Effect.sync(() => {
          view.destroy()
          setup.renderer.destroy()
        }),
    ),
  ))
