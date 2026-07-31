import {
  createTestRenderer,
  MockTreeSitterClient,
} from "@opentui/core/testing"
import { TextAttributes } from "@opentui/core"
import { expect, test } from "bun:test"
import { ToolPresentationView } from "../src/opentui/tool-presentation-view.ts"
import {
  patchFromEditArguments,
  toolPresentation,
} from "../src/ui/tool-presentation.ts"

test("collapses reads and previews writes", () => {
  const read = toolPresentation({
    toolName: "read",
    args: { path: "src/app.ts", offset: 5, limit: 3 },
    result: { content: [{ type: "text", text: "export const value = 1" }] },
  })
  expect(read).toMatchObject({
    canonicalTool: "read",
    title: "read",
    detail: "src/app.ts · lines 5-7",
    state: "complete",
    body: { kind: "none" },
  })

  const markdown = toolPresentation({
    toolName: "read_file",
    args: { file_path: "README.md" },
    result: "# Project",
    pending: true,
  })
  expect(markdown).toMatchObject({
    canonicalTool: "read",
    detail: "README.md",
    state: "pending",
    body: { kind: "none" },
  })

  const write = toolPresentation({
    toolName: "write_file",
    args: { file_path: "src/new.ts", content: "const next = true" },
  })
  expect(write).toMatchObject({
    canonicalTool: "write",
    detail: "src/new.ts",
    body: {
      kind: "code",
      path: "src/new.ts",
      content: "const next = true",
    },
  })

  const longWrite = toolPresentation({
    toolName: "write",
    args: {
      path: "notes.txt",
      content: Array.from({ length: 12 }, (_, index) => `line ${index + 1}`)
        .join("\n"),
    },
  })
  expect(longWrite.body).toEqual({
    kind: "code",
    path: "notes.txt",
    content: [
      "line 1",
      "line 2",
      "line 3",
      "line 4",
      "line 5",
      "line 6",
      "line 7",
      "line 8",
      "line 9",
      "line 10",
      "… (2 more lines, 12 total)",
    ].join("\n"),
    streaming: false,
  })
})

test("builds an edit preview and prefers the exact result patch", () => {
  const args = {
    path: "src/value.ts",
    edits: [
      { oldText: "const value = 1", newText: "const value = 2" },
      { oldText: "old()", newText: "newCall()" },
    ],
  }
  const previewPatch = patchFromEditArguments(args.path, args)
  expect(previewPatch).toContain("--- a/src/value.ts")
  expect(previewPatch).toContain("-const value = 1")
  expect(previewPatch).toContain("+newCall()")

  const preview = toolPresentation({ toolName: "edit_file", args, pending: true })
  expect(preview).toMatchObject({
    canonicalTool: "edit",
    state: "pending",
    body: { kind: "diff", patch: previewPatch, showLineNumbers: false },
  })

  const exactPatch = [
    "--- a/src/value.ts",
    "+++ b/src/value.ts",
    "@@ -8 +8 @@",
    "-const value = 1",
    "+const value = 2",
    "",
  ].join("\n")
  const complete = toolPresentation({
    toolName: "edit",
    args,
    result: {
      content: [{ type: "text", text: "Successfully replaced one block" }],
      details: { patch: exactPatch, diff: "display-only diff" },
    },
  })
  expect(complete).toMatchObject({
    state: "complete",
    body: { kind: "diff", patch: exactPatch, showLineNumbers: true },
  })

  const failed = toolPresentation({
    toolName: "edit",
    args,
    result: { content: [{ type: "text", text: "old text was not unique" }] },
    isError: true,
  })
  expect(failed).toMatchObject({
    state: "error",
    body: { kind: "text", content: "old text was not unique" },
  })
})

test("maps bash output and keeps unknown extension tools visible", () => {
  const bash = toolPresentation({
    toolName: "shell",
    args: { command: "bun test", timeout: 30 },
    result: { content: [{ type: "text", text: "27 pass" }] },
  })
  expect(bash).toMatchObject({
    canonicalTool: "bash",
    title: "$ bun test",
    detail: "timeout 30s",
    body: {
      kind: "text",
      content: "27 pass",
    },
  })

  const longBash = toolPresentation({
    toolName: "bash",
    args: { command: "build" },
    result: Array.from({ length: 9 }, (_, index) => `output ${index + 1}`)
      .join("\n"),
  })
  expect(longBash.body).toEqual({
    kind: "text",
    content: [
      "… (4 earlier lines)",
      "output 5",
      "output 6",
      "output 7",
      "output 8",
      "output 9",
    ].join("\n"),
  })

  const oneLineBash = toolPresentation({
    toolName: "run-command",
    args: { command: "build" },
    result: `${"old".repeat(200)}END`,
  })
  expect(oneLineBash.canonicalTool).toBe("bash")
  expect(oneLineBash.body.kind).toBe("text")
  if (oneLineBash.body.kind === "text") {
    expect(oneLineBash.body.content).toContain("earlier chars")
    expect(oneLineBash.body.content).toEndWith("END")
    expect(oneLineBash.body.content.length).toBeLessThan(500)
  }

  const longCommand = toolPresentation({
    toolName: "bash",
    args: {
      command: [
        "first command",
        "second command",
        "third command",
        "fourth command that must stay hidden",
      ].join("\n"),
    },
  })
  expect(longCommand.title).toContain("first command")
  expect(longCommand.title).toContain("1 more lines")
  expect(longCommand.title).not.toContain("fourth command")

  const wideCommand = toolPresentation({
    toolName: "exec",
    args: { command: "x".repeat(1_000) },
  })
  expect(wideCommand.title).toContain("more chars")
  expect(wideCommand.title.length).toBeLessThan(360)

  const camelCaseRead = toolPresentation({
    toolName: "readFile",
    args: { path: "secret.txt" },
    result: "must stay hidden",
  })
  expect(camelCaseRead).toMatchObject({
    canonicalTool: "read",
    body: { kind: "none" },
  })

  const generic = toolPresentation({
    toolName: "request_user_input",
    args: { question: "Continue?" },
    isError: true,
  })
  expect(generic).toEqual({
    toolName: "request_user_input",
    canonicalTool: "generic",
    title: "request_user_input",
    state: "error",
    body: {
      kind: "text",
      content: '{\n  "question": "Continue?"\n}',
    },
  })

  const completedGeneric = toolPresentation({
    toolName: "web_search",
    args: { query: "OpenTUI" },
    result: { content: [{ type: "text", text: "one result" }] },
  })
  expect(completedGeneric.body).toEqual({
    kind: "text",
    content:
      'input\n{\n  "query": "OpenTUI"\n}\n\noutput\none result',
  })
})

test("renders a write preview and can replace it with an OpenTUI diff", async () => {
  const setup = await createTestRenderer({ width: 70, height: 18 })
  const treeSitterClient = new MockTreeSitterClient()
  treeSitterClient.setMockResult({ highlights: [] })
  const write = toolPresentation({
    toolName: "write",
    args: {
      path: "src/note.ts",
      content: "export const note = 'focused'",
    },
  })
  const view = new ToolPresentationView(setup.renderer, write, {
    treeSitterClient,
  })
  setup.renderer.root.add(view.root)

  try {
    await setup.renderOnce()
    treeSitterClient.resolveAllHighlightOnce()
    await setup.flush()
    let frame = setup.captureCharFrame()
    expect(frame).toContain("write src/note.ts")
    expect(frame).toContain("export const note = 'focused'")

    const edit = toolPresentation({
      toolName: "edit",
      args: {
        path: "src/value.ts",
        edits: [{ oldText: "const value = 1", newText: "const value = 2" }],
      },
      pending: true,
    })
    view.update(write, edit)
    await setup.renderOnce()
    treeSitterClient.resolveAllHighlightOnce()
    await setup.flush()
    frame = setup.captureCharFrame()
    expect(frame).toContain("edit src/value.ts …")
    expect(frame).toContain("const value = 1")
    expect(frame).toContain("const value = 2")
    expect(frame).not.toContain("export const note = 'focused'")
  } finally {
    view.destroy()
    await treeSitterClient.destroy()
    setup.renderer.destroy()
  }
})

test("applies tree-sitter syntax styles to file content", async () => {
  const setup = await createTestRenderer({ width: 60, height: 10 })
  const treeSitterClient = new MockTreeSitterClient()
  treeSitterClient.setMockResult({
    highlights: [[0, 6, "keyword"]],
  })
  const model = toolPresentation({
    toolName: "write",
    args: { path: "src/value.ts", content: "export const value = 1" },
  })
  const view = new ToolPresentationView(setup.renderer, model, {
    treeSitterClient,
  })
  setup.renderer.root.add(view.root)

  try {
    await setup.renderOnce()
    treeSitterClient.resolveAllHighlightOnce()
    await setup.flush()
    const spans = setup.captureSpans().lines.flatMap((line) => line.spans)
    const keyword = spans.find((span) => span.text.includes("export"))
    expect(keyword).toBeDefined()
    expect((keyword?.attributes ?? 0) & TextAttributes.BOLD).not.toBe(0)
  } finally {
    view.destroy()
    await treeSitterClient.destroy()
    setup.renderer.destroy()
  }
})
