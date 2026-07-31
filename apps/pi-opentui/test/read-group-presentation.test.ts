import { expect, test } from "bun:test"
import { readGroupPresentation } from "../src/ui/read-group-presentation.ts"

test("presents one read with its range and text line count", () => {
  const entries = [
    {
      id: "read-1",
      args: { path: "src/app.ts", offset: 5, limit: 3 },
      result: {
        content: [{ type: "text", text: "line one\nline two\n" }],
      },
      pending: false,
      isError: false,
    },
  ] as const

  expect(readGroupPresentation(entries)).toEqual({
    toolName: "read",
    canonicalTool: "read",
    title: "read",
    detail: "src/app.ts:5-7  3 lines",
    state: "complete",
    showStateLabel: false,
    body: { kind: "none" },
  })
})

test("presents grouped progress, failures, ranges, and images", () => {
  const entries = [
    {
      id: "read-1",
      args: { path: "README.md" },
      result: { content: [{ type: "text", text: "one\ntwo" }] },
      pending: false,
      isError: false,
    },
    {
      id: "read-2",
      args: { path: "diagram.png", offset: 2 },
      result: {
        content: [
          { type: "text", text: "image metadata" },
          { type: "image", data: "ignored by presentation" },
        ],
      },
      pending: false,
      isError: false,
    },
    {
      id: "read-3",
      args: { path: "pending.ts", limit: 4 },
      pending: true,
      isError: false,
    },
    {
      id: "read-4",
      args: { path: "missing.ts", offset: 9, limit: 1 },
      result: { content: [{ type: "text", text: "not found" }] },
      pending: false,
      isError: true,
    },
  ] as const

  expect(readGroupPresentation(entries)).toEqual({
    toolName: "read",
    canonicalTool: "read",
    title: "read 4 files",
    detail: "2/4, 1 failed",
    state: "error",
    showStateLabel: false,
    body: {
      kind: "read-group",
      entries: [
        { marker: "✓", path: "README.md", label: "2 lines", state: "complete" },
        { marker: "✓", path: "diagram.png:2", label: "image", state: "complete" },
        { marker: "…", path: "pending.ts:1-4", label: "…", state: "pending" },
        { marker: "✗", path: "missing.ts:9-9", label: "✗", state: "error" },
      ],
    },
  })
})

test("uses the group pending state and generic success label", () => {
  const result = readGroupPresentation([
    {
      id: "read-1",
      args: { path: "one.ts" },
      result: { content: [] },
      pending: false,
      isError: false,
    },
    {
      id: "read-2",
      args: { path: "two.ts" },
      pending: true,
      isError: false,
    },
  ])

  expect(result).toMatchObject({
    title: "read 2 files",
    detail: "1/2",
    state: "pending",
    body: {
      kind: "read-group",
      entries: [
        { marker: "✓", path: "one.ts", label: "✓", state: "complete" },
        { marker: "…", path: "two.ts", label: "…", state: "pending" },
      ],
    },
  })
})

test("rejects an empty read group", () => {
  expect(() => readGroupPresentation([])).toThrow(
    "A read group must contain at least one entry",
  )
})

test("shortens paths below the supplied home directory", () => {
  const result = readGroupPresentation(
    [{
      id: "read-home",
      args: { file_path: "/home/test/project/file.ts" },
      result: { content: [{ type: "text", text: "file" }] },
      pending: false,
      isError: false,
    }],
    { homeDirectory: "/home/test" },
  )
  expect(result.detail).toBe("~/project/file.ts  1 lines")
})
