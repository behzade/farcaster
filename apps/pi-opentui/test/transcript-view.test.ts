import {
  createTestRenderer,
  MockTreeSitterClient,
} from "@opentui/core/testing"
import {
  BoxRenderable,
  MarkdownRenderable,
  RGBA,
  TextRenderable,
} from "@opentui/core"
import { expect, test } from "bun:test"
import { Effect } from "effect"
import {
  TranscriptView,
  workingDots,
} from "../src/opentui/transcript-view.ts"
import { theme } from "../src/opentui/theme.ts"
import type {
  AssistantTranscriptRow,
  TranscriptModel,
  TranscriptRow,
} from "../src/services/transcript.ts"

const row = (id: string, content: string): TranscriptRow => ({
  id,
  kind: "assistant",
  title: "pi",
  content,
  thinking: "",
  thinkingRedacted: false,
  pending: false,
  isError: false,
})

const model = (rows: ReadonlyArray<TranscriptRow>): TranscriptModel => ({
  rows,
  nextRowId: rows.length + 1,
})

test("keeps working dots through model output until activity stops", async () => {
  expect([0, 17, 18, 35, 36, 53, 54].map(workingDots)).toEqual([
    ".",
    ".",
    "..",
    "..",
    "...",
    "...",
    ".",
  ])

  const setup = await createTestRenderer({ width: 40, height: 10 })
  const view = new TranscriptView(setup.renderer)
  setup.renderer.root.add(view.root)
  const pendingRow: AssistantTranscriptRow = {
    id: "assistant-working",
    kind: "assistant",
    title: "pi",
    content: "",
    thinking: "",
    thinkingRedacted: false,
    pending: true,
    isError: false,
  }
  const pending = model([pendingRow])
  const answering = model([
    { ...pendingRow, content: "Answering now" },
  ])

  try {
    view.setWorking(true)
    view.update(undefined, pending)
    await setup.renderOnce()
    const working = view.root.getChildren().at(-1)
    expect(working).toBeInstanceOf(TextRenderable)
    expect((working as TextRenderable).visible).toBe(true)

    view.update(pending, answering)
    await setup.renderOnce()
    expect((working as TextRenderable).visible).toBe(true)
    expect(setup.captureCharFrame()).toContain("Answering now")

    view.setWorking(false)
    await setup.renderOnce()
    expect((working as TextRenderable).visible).toBe(false)
  } finally {
    view.destroy()
    setup.renderer.destroy()
  }
})

test("appends, reorders, and trims owned transcript rows", () =>
  Effect.runPromise(
    Effect.acquireUseRelease(
      Effect.gen(function* () {
        const setup = yield* Effect.tryPromise(() =>
          createTestRenderer({ width: 60, height: 16 })
        )
        const treeSitterClient = new MockTreeSitterClient()
        treeSitterClient.setMockResult({ highlights: [] })
        const view = new TranscriptView(setup.renderer, {
          treeSitterClient,
        })
        setup.renderer.root.add(view.root)
        return { setup, treeSitterClient, view }
      }),
      ({ setup, treeSitterClient, view }) =>
        Effect.gen(function* () {
          const first = model([row("a", "first"), row("b", "second")])
          view.update(undefined, first)
          yield* Effect.tryPromise(() => setup.renderOnce())
          treeSitterClient.resolveAllHighlightOnce()
          yield* Effect.tryPromise(() => setup.flush())
          expect(view.root.getChildrenCount()).toBe(4)

          const reordered = model([
            row("b", "second changed"),
            row("a", "first changed"),
          ])
          view.update(first, reordered)
          yield* Effect.tryPromise(() => setup.renderOnce())
          treeSitterClient.resolveAllHighlightOnce()
          yield* Effect.tryPromise(() => setup.flush())
          const reorderedFrame = setup.captureCharFrame()
          expect(reorderedFrame.indexOf("second changed")).toBeLessThan(
            reorderedFrame.indexOf("first changed"),
          )
          expect(view.root.getChildrenCount()).toBe(4)
          const retainedSecondRow = view.root.getChildren()[1]

          const trimmed = model([
            reordered.rows[0]!,
            row("c", "third"),
          ])
          view.update(reordered, trimmed)
          yield* Effect.tryPromise(() => setup.renderOnce())
          treeSitterClient.resolveAllHighlightOnce()
          yield* Effect.tryPromise(() => setup.flush())
          const trimmedFrame = setup.captureCharFrame()
          expect(trimmedFrame).toContain("second changed")
          expect(trimmedFrame).toContain("third")
          expect(trimmedFrame).not.toContain("first changed")
          expect(view.root.getChildrenCount()).toBe(4)
          expect(view.root.getChildren()[1]).toBe(retainedSecondRow)
        }),
      ({ setup, treeSitterClient, view }) =>
        Effect.promise(async () => {
          view.destroy()
          await treeSitterClient.destroy()
          setup.renderer.destroy()
        }),
    ),
  ))

test("renders thinking, markdown answers, and typed file tools", async () => {
  const setup = await createTestRenderer({ width: 80, height: 28 })
  const treeSitterClient = new MockTreeSitterClient()
  treeSitterClient.setMockResult({ highlights: [] })
  const view = new TranscriptView(setup.renderer, { treeSitterClient })
  setup.renderer.root.add(view.root)

  const transcript: TranscriptModel = {
    rows: [
      {
        id: "assistant-1",
        kind: "assistant",
        title: "pi",
        content: "## Result\n\nThe change is ready.",
        thinking: "Checked the event stream.",
        thinkingRedacted: false,
        pending: false,
        isError: false,
      },
      {
        id: "tool-read-1",
        kind: "tool",
        title: "read",
        content: "export const ready = true",
        pending: false,
        isError: false,
        toolCallId: "read-1",
        toolName: "read",
        args: { path: "src/status.ts", offset: 4, limit: 2 },
        partialResult: undefined,
        result: {
          content: [{ type: "text", text: "export const ready = true" }],
        },
      },
    ],
    nextRowId: 3,
  }

  try {
    view.update(undefined, transcript)
    await setup.renderOnce()
    treeSitterClient.resolveAllHighlightOnce()
    await setup.flush()
    const frame = setup.captureCharFrame()
    expect(frame).toContain("Checked the event stream.")
    expect(frame).toContain("Result")
    expect(frame).toContain("The change is ready.")
    expect(frame).toContain("read src/status.ts:4-5  1 lines")
    expect(frame).not.toContain("export const ready = true")

    view.setHideThinkingBlock(true)
    await setup.renderOnce()
    treeSitterClient.resolveAllHighlightOnce()
    await setup.flush()
    const hiddenFrame = setup.captureCharFrame()
    expect(hiddenFrame).toContain("Thinking…")
    expect(hiddenFrame).not.toContain("Checked the event stream.")
  } finally {
    view.destroy()
    await treeSitterClient.destroy()
    setup.renderer.destroy()
  }
})

test("uses a user band while tools and model prose keep a clear background", async () => {
  const setup = await createTestRenderer({ width: 72, height: 30 })
  const treeSitterClient = new MockTreeSitterClient()
  treeSitterClient.setMockResult({ highlights: [] })
  const view = new TranscriptView(setup.renderer, { treeSitterClient })
  setup.renderer.root.add(view.root)

  const transcript: TranscriptModel = {
    rows: [
      {
        id: "user-1",
        kind: "user",
        title: "you",
        content: "Inspect the project",
        pending: false,
        isError: false,
      },
      {
        id: "tool-1",
        kind: "tool",
        title: "bash",
        content: "done",
        pending: false,
        isError: false,
        toolCallId: "bash-1",
        toolName: "bash",
        args: { command: "find src -type f" },
        partialResult: undefined,
        result: "src/main.ts",
      },
      {
        id: "assistant-1",
        kind: "assistant",
        title: "pi",
        content: "The project has one source file.",
        thinking: "Checked the file list.",
        thinkingRedacted: false,
        pending: false,
        isError: false,
      },
    ],
    nextRowId: 4,
  }

  try {
    view.update(undefined, transcript)
    await setup.renderOnce()
    treeSitterClient.resolveAllHighlightOnce()
    await setup.flush()

    const frame = setup.captureCharFrame()
    expect(frame).toContain("Inspect the project")
    expect(frame).toContain("$ find src -type f")
    expect(frame).toContain("src/main.ts")
    expect(frame).toContain("Checked the file list.")
    expect(frame).toContain("The project has one source file.")
    expect(frame).not.toContain("you\n")
    expect(frame).not.toContain("pi\n")

    const captured = setup.captureSpans()
    const userBg = RGBA.fromHex(theme.userMessageBg)
    const userLine = captured.lines.find((line) =>
      line.spans.some((span) => span.text.includes("Inspect the project"))
    )
    const toolLine = captured.lines.find((line) =>
      line.spans.some((span) => span.text.includes("find src -type f"))
    )
    const answerLine = captured.lines.find((line) =>
      line.spans.some((span) =>
        span.text.includes("The project has one source file.")
      )
    )
    expect(userLine?.spans.some((span) => span.bg.equals(userBg))).toBe(true)
    expect(toolLine?.spans.some((span) => span.bg.equals(userBg))).toBe(false)
    expect(answerLine?.spans.some((span) => span.bg.equals(userBg))).toBe(false)
  } finally {
    view.destroy()
    await treeSitterClient.destroy()
    setup.renderer.destroy()
  }
})

test("settles thinking and adds answer spacing when a turn completes", async () => {
  const setup = await createTestRenderer({ width: 60, height: 16 })
  const treeSitterClient = new MockTreeSitterClient()
  treeSitterClient.setMockResult({ highlights: [] })
  const view = new TranscriptView(setup.renderer, { treeSitterClient })
  setup.renderer.root.add(view.root)

  const pendingRow: AssistantTranscriptRow = {
    id: "assistant-1",
    kind: "assistant",
    title: "pi",
    content: "",
    thinking: "Inspecting files",
    thinkingRedacted: false,
    pending: true,
    isError: false,
  }
  const pending = model([pendingRow])
  const stillPendingRow: AssistantTranscriptRow = {
    ...pendingRow,
    content: "**Draft answer**",
    thinking: "Inspecting files now",
  }
  const stillPending = model([stillPendingRow])
  const complete = model([{
    ...stillPendingRow,
    content: "**Inspection complete**",
    pending: false,
  }])

  try {
    view.update(undefined, pending)
    await setup.renderOnce()
    treeSitterClient.resolveAllHighlightOnce()
    await setup.flush()

    const pendingAssistant = view.root.getChildren()[1] as BoxRenderable
    const pendingThinkingRoot = pendingAssistant.getChildren()[0] as BoxRenderable
    const pendingAnswerRoot = pendingAssistant.getChildren()[1] as BoxRenderable
    const streamingThinking = pendingThinkingRoot.getChildren()[0]
    const streamingAnswer = pendingAnswerRoot.getChildren()[0]
    expect(streamingThinking).toBeInstanceOf(TextRenderable)
    expect(streamingAnswer).toBeInstanceOf(TextRenderable)

    view.update(pending, stillPending)
    await setup.renderOnce()
    await setup.flush()
    expect(pendingThinkingRoot.getChildren()[0]).toBe(streamingThinking)
    expect(pendingAnswerRoot.getChildren()[0]).toBe(streamingAnswer)
    expect(setup.captureCharFrame()).toContain("**Draft answer**")

    view.update(stillPending, complete)
    await setup.renderOnce()
    treeSitterClient.resolveAllHighlightOnce()
    await setup.flush()

    const assistant = view.root.getChildren()[1] as BoxRenderable
    const thinkingRoot = assistant.getChildren()[0] as BoxRenderable
    const answerRoot = assistant.getChildren()[1] as BoxRenderable
    const thinking = thinkingRoot.getChildren()[0]
    const answer = answerRoot.getChildren()[0]
    expect(thinking).toBeInstanceOf(MarkdownRenderable)
    expect((thinking as MarkdownRenderable).streaming).toBe(false)
    expect(answer).toBeInstanceOf(MarkdownRenderable)
    expect(thinking).not.toBe(streamingThinking)
    expect(answer).not.toBe(streamingAnswer)

    const lines = setup.captureCharFrame().split("\n")
    const thinkingLine = lines.findIndex((line) =>
      line.includes("Inspecting files now")
    )
    const answerLine = lines.findIndex((line) =>
      line.includes("Inspection complete")
    )
    expect(answerLine - thinkingLine).toBeGreaterThan(1)
  } finally {
    view.destroy()
    await treeSitterClient.destroy()
    setup.renderer.destroy()
  }
})

test("renders one live card for a grouped read run", async () => {
  const setup = await createTestRenderer({ width: 72, height: 22 })
  const treeSitterClient = new MockTreeSitterClient()
  treeSitterClient.setMockResult({ highlights: [] })
  const view = new TranscriptView(setup.renderer, { treeSitterClient })
  setup.renderer.root.add(view.root)

  const readRows = (pending: boolean): ReadonlyArray<TranscriptRow> => [
    {
      id: "tool-read-1",
      kind: "tool",
      title: "read",
      content: pending ? "" : "one\ntwo",
      pending,
      isError: false,
      toolCallId: "read-1",
      toolName: "read",
      args: { path: "README.md" },
      partialResult: undefined,
      result: pending
        ? undefined
        : { content: [{ type: "text", text: "one\ntwo" }] },
      readGroupId: "read-1",
    },
    {
      id: "tool-read-2",
      kind: "tool",
      title: "read",
      content: pending ? "" : "export const ready = true",
      pending,
      isError: false,
      toolCallId: "read-2",
      toolName: "read",
      args: { path: "src/status.ts", offset: 4, limit: 2 },
      partialResult: undefined,
      result: pending
        ? undefined
        : {
            content: [{ type: "text", text: "export const ready = true" }],
          },
      readGroupId: "read-1",
    },
  ]
  const pending = model(readRows(true))
  const complete = model(readRows(false))

  try {
    view.update(undefined, pending)
    await setup.renderOnce()
    await setup.flush()
    expect(view.root.getChildrenCount()).toBe(3)
    const groupRoot = view.root.getChildren()[1] as BoxRenderable
    let frame = setup.captureCharFrame()
    expect(frame).toContain("read 2 files 0/2")
    expect(frame).toContain("… README.md  …")
    expect(frame).toContain("… src/status.ts:4-5  …")

    view.update(pending, complete)
    await setup.renderOnce()
    await setup.flush()
    frame = setup.captureCharFrame()
    expect(view.root.getChildren()[1]).toBe(groupRoot)
    expect(view.root.getChildrenCount()).toBe(3)
    expect(frame).toContain("read 2 files 2/2")
    expect(frame).toContain("✓ README.md  2 lines")
    expect(frame).toContain("✓ src/status.ts:4-5  1 lines")

    const readLine = setup.captureSpans().lines.find((line) =>
      line.spans.some((span) => span.text.includes("README.md"))
    )
    const success = RGBA.fromHex(theme.assistant)
    const path = RGBA.fromHex(theme.user)
    const muted = RGBA.fromHex(theme.muted)
    expect(
      readLine?.spans.some(
        (span) => span.text.includes("✓") && span.fg.equals(success),
      ),
    ).toBe(true)
    expect(
      readLine?.spans.some(
        (span) => span.text.includes("README.md") && span.fg.equals(path),
      ),
    ).toBe(true)
    expect(
      readLine?.spans.some(
        (span) => span.text.includes("2 lines") && span.fg.equals(muted),
      ),
    ).toBe(true)

    const groupBody = groupRoot.getChildren()[1]
    const answerRow = row("assistant-after", "First answer")
    const withAnswer = model([...complete.rows, answerRow])
    view.update(complete, withAnswer)
    const changedAnswer = model([
      ...complete.rows,
      { ...answerRow, content: "Second answer" },
    ])
    view.update(withAnswer, changedAnswer)
    expect(groupRoot.getChildren()[1]).toBe(groupBody)
  } finally {
    view.destroy()
    await treeSitterClient.destroy()
    setup.renderer.destroy()
  }
})
