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
