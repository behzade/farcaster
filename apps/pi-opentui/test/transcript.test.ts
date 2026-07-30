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
  const assistantId = started.rows[0]?.id
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
  })
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
      content: [{ type: "text", text: "saved answer" }],
    },
    {
      role: "assistant",
      content: [{ type: "toolCall", name: "read" }],
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
    "$ pwd\n/work",
    "shown",
  ])
  expect(transcript.rows.map((row) => row.title)).toEqual([
    "you",
    "compaction summary",
    "pi",
    "read",
    "shell",
    "note",
  ])
})
