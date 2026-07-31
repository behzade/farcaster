import type {
  AgentSessionEvent,
} from "@earendil-works/pi-coding-agent"
import { expect, test } from "bun:test"
import {
  emptyTranscript,
  reduceTranscriptEvent,
  transcriptFromMessages,
} from "../src/services/transcript.ts"

const event = (value: unknown): AgentSessionEvent =>
  value as AgentSessionEvent

test("opens a pending assistant row before model output", () => {
  const started = reduceTranscriptEvent(
    emptyTranscript,
    event({ type: "turn_start" }),
  )
  const assistantId = started.rows.find(
    (row) => row.kind === "assistant" && row.pending,
  )?.id

  expect(started.rows).toEqual([
    expect.objectContaining({
      id: assistantId,
      kind: "assistant",
      content: "",
      thinking: "",
      pending: true,
    }),
  ])

  const messageStarted = reduceTranscriptEvent(
    started,
    event({
      type: "message_start",
      message: { role: "assistant", content: [] },
    }),
  )
  expect(messageStarted.rows).toHaveLength(1)
  expect(
    messageStarted.rows.find(
      (row) => row.kind === "assistant" && row.pending,
    )?.id,
  ).toBe(assistantId)
})

test("keeps stable rows through assistant and tool updates", () => {
  const started = reduceTranscriptEvent(
    emptyTranscript,
    event({
      type: "message_start",
      message: {
        role: "assistant",
        content: [{ type: "text", text: "" }],
      },
    }),
  )
  const updated = reduceTranscriptEvent(
    started,
    event({
      type: "message_update",
      message: {
        role: "assistant",
        content: [{ type: "text", text: "Hello" }],
      },
    }),
  )
  const assistantId = updated.rows[0]?.id
  const ended = reduceTranscriptEvent(
    updated,
    event({
      type: "message_end",
      message: {
        role: "assistant",
        content: [{ type: "text", text: "Hello world" }],
      },
    }),
  )
  const toolStarted = reduceTranscriptEvent(
    ended,
    event({
      type: "tool_execution_start",
      toolCallId: "call-1",
      toolName: "read",
      args: { path: "README.md" },
    }),
  )
  const toolEnded = reduceTranscriptEvent(
    toolStarted,
    event({
      type: "tool_execution_end",
      toolCallId: "call-1",
      toolName: "read",
      result: {
        content: [{ type: "text", text: "file contents" }],
      },
      isError: false,
    }),
  )

  expect(ended.rows[0]).toMatchObject({
    id: assistantId,
    content: "Hello world",
    pending: false,
  })
  expect(toolEnded.rows[1]).toMatchObject({
    id: "tool-call-1",
    title: "read",
    content: "file contents",
    pending: false,
    isError: false,
    args: { path: "README.md" },
    result: {
      content: [{ type: "text", text: "file contents" }],
    },
  })
})

test("keeps streamed thinking beside the final answer", () => {
  const started = reduceTranscriptEvent(
    emptyTranscript,
    event({
      type: "message_start",
      message: { role: "assistant", content: [] },
    }),
  )
  const thinking = reduceTranscriptEvent(
    started,
    event({
      type: "message_update",
      message: {
        role: "assistant",
        content: [{ type: "thinking", thinking: "Check the types." }],
      },
      assistantMessageEvent: { type: "thinking_delta", delta: "types." },
    }),
  )
  const answering = reduceTranscriptEvent(
    thinking,
    event({
      type: "message_update",
      message: {
        role: "assistant",
        content: [
          { type: "thinking", thinking: "Check the types." },
          { type: "text", text: "Done." },
        ],
      },
      assistantMessageEvent: { type: "text_delta", delta: "Done." },
    }),
  )

  expect(answering.rows).toHaveLength(1)
  expect(answering.rows[0]).toMatchObject({
    id: thinking.rows[0]?.id,
    kind: "assistant",
    thinking: "Check the types.",
    content: "Done.",
    pending: true,
  })
})

test("does not expose redacted thinking signatures", () => {
  const transcript = transcriptFromMessages([
    {
      role: "assistant",
      content: [
        { type: "thinking", thinking: "Visible summary" },
        {
          type: "thinking",
          thinking: "",
          thinkingSignature: "private-provider-payload",
          redacted: true,
        },
        { type: "text", text: "Safe answer" },
      ],
    },
  ])

  expect(transcript.rows[0]).toMatchObject({
    kind: "assistant",
    content: "Safe answer",
    thinking: "Visible summary",
    thinkingRedacted: true,
  })
  expect(JSON.stringify(transcript)).not.toContain("private-provider-payload")
})

test("keeps only the grouped tool row for a tool-only turn", () => {
  const started = reduceTranscriptEvent(
    emptyTranscript,
    event({
      type: "message_start",
      message: { role: "assistant", content: [] },
    }),
  )
  const toolCall = {
    role: "assistant",
    content: [
      {
        type: "toolCall",
        id: "call-only",
        name: "read",
        arguments: { path: "README.md" },
      },
    ],
  }
  const updated = reduceTranscriptEvent(
    started,
    event({
      type: "message_update",
      message: toolCall,
      assistantMessageEvent: { type: "toolcall_end" },
    }),
  )
  const ended = reduceTranscriptEvent(
    updated,
    event({ type: "message_end", message: toolCall }),
  )

  expect(ended.rows).toHaveLength(1)
  expect(ended.rows[0]).toMatchObject({
    id: "tool-call-only",
    kind: "tool",
    pending: true,
    readGroupId: "call-only",
  })
})

test("keeps read groups within source assistant content boundaries", () => {
  const message = {
    role: "assistant",
    content: [
      {
        type: "toolCall",
        id: "read-a",
        name: "read",
        arguments: { path: "a.ts" },
      },
      {
        type: "toolCall",
        id: "read-b",
        name: "read",
        arguments: { path: "b.ts" },
      },
      { type: "text", text: "separator" },
      {
        type: "toolCall",
        id: "read-c",
        name: "read",
        arguments: { path: "c.ts" },
      },
      { type: "thinking", thinking: "separator" },
      {
        type: "toolCall",
        id: "read-d",
        name: "read",
        arguments: { path: "d.ts" },
      },
      {
        type: "toolCall",
        id: "bash-1",
        name: "bash",
        arguments: { command: "pwd" },
      },
      {
        type: "toolCall",
        id: "read-e",
        name: "read",
        arguments: { path: "e.ts" },
      },
    ],
  }
  const ended = reduceTranscriptEvent(
    emptyTranscript,
    event({ type: "message_end", message }),
  )
  const tools = ended.rows.filter((row) => row.kind === "tool")
  expect(tools.map((row) => [row.toolCallId, row.readGroupId])).toEqual([
    ["read-a", "read-a"],
    ["read-b", "read-a"],
    ["read-c", "read-c"],
    ["read-d", "read-d"],
    ["bash-1", undefined],
    ["read-e", "read-e"],
  ])

  const startedFollower = reduceTranscriptEvent(
    ended,
    event({
      type: "tool_execution_start",
      toolCallId: "read-b",
      toolName: "read",
      args: { path: "b.ts", offset: 2 },
    }),
  )
  expect(
    startedFollower.rows.find(
      (row) => row.kind === "tool" && row.toolCallId === "read-b",
    ),
  ).toMatchObject({
    args: { path: "b.ts", offset: 2 },
    readGroupId: "read-a",
  })
})

test("marks an unresolved saved tool call as stopped", () => {
  const transcript = transcriptFromMessages([
    {
      role: "assistant",
      content: [
        {
          type: "toolCall",
          id: "orphan-call",
          name: "write",
          arguments: { path: "unfinished.ts", content: "draft" },
        },
      ],
    },
  ])

  expect(transcript.rows).toHaveLength(1)
  expect(transcript.rows[0]).toMatchObject({
    kind: "tool",
    pending: false,
    isError: true,
    content: "Tool did not finish",
  })
})

test("bounds tool payloads and replaces duplicate starts", () => {
  const first = reduceTranscriptEvent(
    emptyTranscript,
    event({
      type: "tool_execution_start",
      toolCallId: "large-call",
      toolName: "write",
      args: { path: "large.txt", content: "x".repeat(20_000) },
    }),
  )
  const restarted = reduceTranscriptEvent(
    first,
    event({
      type: "tool_execution_start",
      toolCallId: "large-call",
      toolName: "write",
      args: { path: "large.txt", content: "y".repeat(20_000) },
    }),
  )
  const ended = reduceTranscriptEvent(
    restarted,
    event({
      type: "tool_execution_end",
      toolCallId: "large-call",
      toolName: "write",
      result: {
        content: [{ type: "text", text: "z".repeat(20_000) }],
      },
      isError: false,
    }),
  )

  expect(restarted.rows).toHaveLength(1)
  expect(JSON.stringify(restarted.rows[0]).length).toBeLessThan(9_000)
  expect(JSON.stringify(ended.rows[0]).length).toBeLessThan(18_000)
  expect(JSON.stringify(ended.rows[0])).not.toContain("z".repeat(9_000))

  const wide = reduceTranscriptEvent(
    emptyTranscript,
    event({
      type: "tool_execution_start",
      toolCallId: "wide-call",
      toolName: "extension",
      args: Array.from({ length: 50_000 }, () => ({})),
    }),
  )
  let deep: Record<string, unknown> = {}
  const root = deep
  for (let index = 0; index < 10_000; index += 1) {
    const child: Record<string, unknown> = {}
    deep.child = child
    deep = child
  }
  const deeplyNested = reduceTranscriptEvent(
    emptyTranscript,
    event({
      type: "tool_execution_start",
      toolCallId: "deep-call",
      toolName: "extension",
      args: root,
    }),
  )
  expect(JSON.stringify(wide.rows[0]).length).toBeLessThan(5_000)
  expect(JSON.stringify(deeplyNested.rows[0])).toContain("[max depth]")
})

test("shows retry and compaction state", () => {
  const retrying = reduceTranscriptEvent(
    emptyTranscript,
    event({
      type: "auto_retry_start",
      attempt: 2,
      maxAttempts: 3,
      delayMs: 500,
      errorMessage: "fetch failed",
    }),
  )
  const compacting = reduceTranscriptEvent(
    retrying,
    event({
      type: "compaction_start",
      reason: "threshold",
    }),
  )
  const compacted = reduceTranscriptEvent(
    compacting,
    event({
      type: "compaction_end",
      aborted: false,
    }),
  )

  expect(compacted.rows.map((row) => row.content)).toEqual([
    "Retry 2/3 in 500ms: fetch failed",
    "Compaction started (threshold)",
    "Compaction finished",
  ])
})

test("rebuilds transcript rows from saved messages", () => {
  const transcript = transcriptFromMessages([
    { role: "user", content: "saved question" },
    {
      role: "compactionSummary",
      summary: "Earlier work",
      tokensBefore: 10,
    },
    {
      role: "assistant",
      content: [
        { type: "thinking", thinking: "saved thought" },
        { type: "text", text: "saved answer" },
        {
          type: "toolCall",
          id: "saved-call",
          name: "read",
          arguments: { path: "saved.ts", offset: 4 },
        },
      ],
    },
    {
      role: "toolResult",
      toolCallId: "saved-call",
      toolName: "read",
      content: [{ type: "text", text: "saved file" }],
      isError: false,
    },
    {
      role: "bashExecution",
      command: "pwd",
      output: "/work",
      exitCode: 0,
    },
    {
      role: "custom",
      customType: "note",
      content: "shown",
      display: true,
    },
    {
      role: "custom",
      customType: "state",
      content: "hidden",
      display: false,
    },
  ])

  expect(transcript.rows.map((row) => row.content)).toEqual([
    "saved question",
    "Earlier work",
    "saved answer",
    "saved file",
    "/work",
    "shown",
  ])
  expect(transcript.rows.map((row) => row.title)).toEqual([
    "you",
    "compaction summary",
    "pi",
    "read",
    "bash",
    "note",
  ])
  expect(transcript.rows[2]).toMatchObject({
    kind: "assistant",
    thinking: "saved thought",
  })
  expect(transcript.rows[3]).toMatchObject({
    kind: "tool",
    args: { path: "saved.ts", offset: 4 },
    pending: false,
  })
})

test("restores consecutive read calls as one source group", () => {
  const transcript = transcriptFromMessages([
    {
      role: "assistant",
      content: [
        {
          type: "toolCall",
          id: "saved-a",
          name: "read",
          arguments: { path: "a.ts" },
        },
        {
          type: "toolCall",
          id: "saved-b",
          name: "read_file",
          arguments: { file_path: "b.ts" },
        },
      ],
    },
    {
      role: "toolResult",
      toolCallId: "saved-a",
      toolName: "read",
      content: [{ type: "text", text: "a" }],
      isError: false,
    },
    {
      role: "toolResult",
      toolCallId: "saved-b",
      toolName: "read_file",
      content: [{ type: "text", text: "b" }],
      isError: false,
    },
  ])

  expect(transcript.rows).toHaveLength(2)
  expect(transcript.rows).toMatchObject([
    { kind: "tool", toolCallId: "saved-a", readGroupId: "saved-a" },
    {
      kind: "tool",
      toolCallId: "saved-b",
      readGroupId: "saved-a",
      args: { file_path: "b.ts" },
    },
  ])
})
