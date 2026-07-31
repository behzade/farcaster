import type { ToolDefinition } from "@earendil-works/pi-coding-agent"
import { expect, test } from "bun:test"
import { createExtensionToolPresenter } from "../src/services/extension-tool-presentation.ts"

const textComponent = (text: string) => ({
  render: () => [text],
  invalidate: () => undefined,
})

test("uses an extension tool's call and result renderers", () => {
  let resultSawCallState = false
  let callSawPreviousComponent = false
  const definition = {
    name: "web_search",
    label: "Web search",
    renderCall: (args: { query: string }, _theme: unknown, context: {
      state: Record<string, unknown>
      lastComponent: unknown
    }) => {
      callSawPreviousComponent ||= context.lastComponent !== undefined
      context.state.query = args.query
      return textComponent(`search ${args.query}`)
    },
    renderResult: (_result: unknown, _options: unknown, _theme: unknown, context: {
      state: Record<string, unknown>
    }) => {
      resultSawCallState = context.state.query === "OpenTUI"
      return textComponent("2 results\n1. OpenTUI docs")
    },
  } as unknown as ToolDefinition
  const present = createExtensionToolPresenter({
    cwd: "/work",
    getDefinition: (name) => name === "web_search" ? definition : undefined,
  })

  expect(present({
    toolCallId: "call-1",
    toolName: "web_search",
    args: { query: "OpenTUI" },
    result: undefined,
    pending: true,
    isError: false,
  })).toEqual({ label: "Web search", call: "search OpenTUI" })

  expect(present({
    toolCallId: "call-1",
    toolName: "web_search",
    args: { query: "OpenTUI" },
    result: { content: [{ type: "text", text: "raw result" }] },
    pending: false,
    isError: false,
  })).toEqual({
    label: "Web search",
    call: "search OpenTUI",
    result: "2 results\n1. OpenTUI docs",
  })
  expect(callSawPreviousComponent).toBe(true)
  expect(resultSawCallState).toBe(true)
})

test("a broken extension renderer cannot break event handling", () => {
  const definition = {
    name: "broken",
    label: "Broken tool",
    renderCall: () => {
      throw new Error("display failed")
    },
  } as unknown as ToolDefinition
  const present = createExtensionToolPresenter({
    cwd: "/work",
    getDefinition: () => definition,
  })

  expect(present({
    toolCallId: "call-2",
    toolName: "broken",
    args: {},
    result: undefined,
    pending: false,
    isError: false,
  })).toEqual({ label: "Broken tool" })
})
