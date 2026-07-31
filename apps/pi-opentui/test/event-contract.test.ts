import type { AgentSessionEvent } from "@earendil-works/pi-coding-agent"
import { expect, test } from "bun:test"
import { assertAgentSessionEventContract } from "../src/services/event-contract.ts"
import {
  emptyTranscript,
  reduceTranscriptEvent,
} from "../src/services/transcript.ts"

test("accepts an event in the installed Pi contract", () => {
  expect(() =>
    assertAgentSessionEventContract({ type: "agent_settled" }),
  ).not.toThrow()
})

test("fails closed on an unknown runtime event type", () => {
  const futureEvent = {
    type: "future_event",
  } as unknown as AgentSessionEvent

  expect(() => assertAgentSessionEventContract(futureEvent)).toThrow(
    "Unhandled AgentSessionEvent type: future_event",
  )
  expect(() => reduceTranscriptEvent(emptyTranscript, futureEvent)).toThrow(
    "Unhandled AgentSessionEvent type: future_event",
  )
})
