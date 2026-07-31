import type { SessionStats } from "@earendil-works/pi-coding-agent"
import { expect, test } from "bun:test"
import {
  applyAppStateUpdate,
  type AppSnapshot,
} from "../src/services/app-state-model.ts"
import { emptyLiveUsage } from "../src/services/live-usage.ts"
import { emptyTranscript } from "../src/services/transcript.ts"

const sessionStats: SessionStats = {
  sessionFile: undefined,
  sessionId: "session-1",
  userMessages: 0,
  assistantMessages: 0,
  toolCalls: 0,
  toolResults: 0,
  totalMessages: 0,
  tokens: {
    input: 0,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    total: 0,
  },
  cost: 0,
}

const snapshot = (phase: AppSnapshot["phase"]): AppSnapshot => ({
  cwd: "/work",
  hideThinkingBlock: false,
  phase,
  activeTools: [],
  model: undefined,
  thinkingLevel: "off",
  sessionStats,
  liveUsage: emptyLiveUsage,
  extensionPaths: [],
  extensionErrors: [],
  eventCount: 0,
  lastEvent: undefined,
  error: phase === "fatal" ? "unknown event" : undefined,
  transcript: emptyTranscript,
  dialog: undefined,
  authNotice: undefined,
  statuses: {},
  commands: [],
  promptQueue: { steering: [], followUp: [] },
  draftRestore: undefined,
})

test("applies normal updates but freezes every field after a fatal event", () => {
  const ready = snapshot("ready")
  const updated = applyAppStateUpdate(ready, (current) => ({
    ...current,
    phase: "running",
    eventCount: 1,
  }))
  expect(updated.phase).toBe("running")
  expect(updated.eventCount).toBe(1)

  const fatal = snapshot("fatal")
  const lateUpdate = applyAppStateUpdate(fatal, (current) => ({
    ...current,
    phase: "ready",
    error: undefined,
    eventCount: 99,
    transcript: {
      ...current.transcript,
      nextRowId: 99,
    },
  }))
  expect(lateUpdate).toBe(fatal)
})
