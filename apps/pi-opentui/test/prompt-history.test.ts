import { expect, test } from "bun:test"
import { PromptHistory } from "../src/services/prompt-history.ts"

test("moves through prompts and restores the draft", () => {
  const history = new PromptHistory()
  history.add("first")
  history.add("second")

  expect(
    history.navigate("older", { text: "draft", cursorOffset: 2 }),
  ).toEqual({ text: "second", cursorOffset: 0 })
  expect(
    history.navigate("older", { text: "second", cursorOffset: 0 }),
  ).toEqual({ text: "first", cursorOffset: 0 })
  expect(
    history.navigate("newer", { text: "first", cursorOffset: 0 }),
  ).toEqual({ text: "second", cursorOffset: 6 })
  expect(
    history.navigate("newer", { text: "second", cursorOffset: 6 }),
  ).toEqual({ text: "draft", cursorOffset: 2 })
  expect(history.isBrowsing).toBe(false)
})

test("skips blanks and adjacent duplicates", () => {
  const history = new PromptHistory()
  history.add("  ")
  history.add("same")
  history.add(" same ")

  expect(
    history.navigate("older", { text: "", cursorOffset: 0 }),
  ).toEqual({ text: "same", cursorOffset: 0 })
  expect(
    history.navigate("older", { text: "same", cursorOffset: 0 }),
  ).toBeUndefined()
})
