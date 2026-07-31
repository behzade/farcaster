import { expect, test } from "bun:test"
import { readFile } from "node:fs/promises"

test("the process boundary does not interpret app commands or Pi events", async () => {
  const source = await readFile(
    new URL("../src/main.ts", import.meta.url),
    "utf8",
  )

  expect(source).not.toMatch(/command\.(?:_tag|name)/)
  expect(source).not.toMatch(/event\.type/)
})

test("only the activity reducer constructs lifecycle state", async () => {
  const files = [
    "app-state.ts",
    "model-actions.ts",
    "session-actions.ts",
    "reload-action.ts",
    "compaction-actions.ts",
    "prompt-queue-actions.ts",
  ]
  const source = (
    await Promise.all(
      files.map((file) =>
        readFile(
          new URL(`../src/services/${file}`, import.meta.url),
          "utf8",
        ),
      ),
    )
  ).join("\n")

  expect(source).not.toMatch(/\bphase\s*:/)
  expect(source).not.toMatch(/\.phase\b/)
  expect(source).not.toMatch(/\bisCompacting\b|\bpromptWasAborted\b/)
  expect(source).not.toMatch(/activity\s*:\s*\{\s*_tag\s*:/)
})
