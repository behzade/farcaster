import type { AgentSessionEvent } from "@earendil-works/pi-coding-agent"
import { expect, test } from "bun:test"
import {
  activityError,
  activityStatus,
  awaitingModelActivity,
  canAcceptInput,
  canInterrupt,
  canStartPrompt,
  commandActivity,
  fatalActivity,
  idleActivity,
  promptRoute,
  reduceActivity,
  showsAgentWorking,
  type AppActivity,
} from "../src/services/app-activity.ts"

const event = (value: object): AgentSessionEvent => value as AgentSessionEvent

const sessionEvent = (activity: AppActivity, value: object): AppActivity =>
  reduceActivity(activity, {
    _tag: "SessionEvent",
    event: event(value),
  })

test("derives all user rules from one activity value", () => {
  const cases: ReadonlyArray<
    readonly [AppActivity, string, boolean, boolean, boolean]
  > = [
    [idleActivity, "ready", true, true, false],
    [awaitingModelActivity(), "waiting", false, true, true],
    [commandActivity("model"), "model", false, false, false],
    [commandActivity("login"), "login", false, false, true],
    [{ _tag: "Compacting", reason: "manual" }, "compacting", false, true, true],
    [{ _tag: "Stopping", target: "turn" }, "stopping", false, false, false],
    [{ _tag: "Failed", message: "bad" }, "error", true, true, false],
    [fatalActivity("unknown event"), "fatal", false, false, false],
  ]

  for (const [activity, status, start, input, interrupt] of cases) {
    expect(activityStatus(activity)).toBe(status)
    expect(canStartPrompt(activity)).toBe(start)
    expect(canAcceptInput(activity)).toBe(input)
    expect(canInterrupt(activity)).toBe(interrupt)
  }
  expect(activityError(fatalActivity("unknown event"))).toBe("unknown event")
  expect(activityError(idleActivity)).toBeUndefined()
  expect(promptRoute(idleActivity)).toBe("start")
  expect(promptRoute(awaitingModelActivity())).toBe("steer")
  expect(promptRoute({ _tag: "Compacting", reason: "manual" })).toBe(
    "after-compaction",
  )
  expect(promptRoute(commandActivity("login"))).toBe("reject")
  expect(showsAgentWorking(awaitingModelActivity())).toBe(true)
  expect(
    showsAgentWorking({ _tag: "Turn", stage: { _tag: "RunningTools" } }),
  ).toBe(true)
  expect(showsAgentWorking({ _tag: "Stopping", target: "turn" })).toBe(true)
  expect(showsAgentWorking(idleActivity)).toBe(false)
})

test("folds a turn through waiting, text, tools, retry, and settle", () => {
  const waiting = sessionEvent(
    idleActivity,
    { type: "turn_start" },
  )
  expect(waiting).toEqual(awaitingModelActivity())

  const streaming = sessionEvent(
    waiting,
    { type: "message_update", message: { role: "assistant" } },
  )
  expect(streaming).toEqual({
    _tag: "Turn",
    stage: { _tag: "Streaming" },
  })

  const tools = sessionEvent(
    streaming,
    { type: "tool_execution_start" },
  )
  expect(tools).toEqual({
    _tag: "Turn",
    stage: { _tag: "RunningTools" },
  })

  const retrying = sessionEvent(
    tools,
    {
      type: "auto_retry_start",
      attempt: 2,
      maxAttempts: 3,
      delayMs: 500,
    },
  )
  expect(activityStatus(retrying)).toBe("retry 2/3")
  expect(
    sessionEvent(retrying, { type: "agent_settled" }),
  ).toEqual(idleActivity)
})

test("does not let late turn events end commands or compaction", () => {
  const model = commandActivity("model")
  expect(
    sessionEvent(model, { type: "agent_settled" }),
  ).toBe(model)
  expect(
    sessionEvent(
      model,
      { type: "message_start", message: { role: "assistant" } },
    ),
  ).toBe(model)

  const compacting: AppActivity = {
    _tag: "Compacting",
    reason: "manual",
  }
  expect(
    sessionEvent(compacting, { type: "agent_settled" }),
  ).toBe(compacting)
})

test("stopping keeps its target until that target ends", () => {
  const stoppingTurn: AppActivity = { _tag: "Stopping", target: "turn" }
  expect(
    sessionEvent(
      stoppingTurn,
      { type: "message_update", message: { role: "assistant" } },
    ),
  ).toBe(stoppingTurn)
  expect(
    sessionEvent(stoppingTurn, { type: "agent_settled" }),
  ).toEqual(idleActivity)

  const stoppingCompaction: AppActivity = {
    _tag: "Stopping",
    target: "compaction",
  }
  expect(
    sessionEvent(
      stoppingCompaction,
      { type: "agent_settled" },
    ),
  ).toBe(stoppingCompaction)
  expect(
    sessionEvent(
      stoppingCompaction,
      { type: "compaction_end", willRetry: false },
    ),
  ).toEqual(idleActivity)
})

test("only compaction can start its queued continuation", () => {
  expect(
    reduceActivity(idleActivity, { _tag: "ContinueAfterCompaction" }),
  ).toEqual(idleActivity)
  expect(
    reduceActivity(
      { _tag: "Compacting", reason: "manual" },
      { _tag: "ContinueAfterCompaction" },
    ),
  ).toEqual(awaitingModelActivity())
})

test("rejects actions that do not fit the current state", () => {
  const turn = awaitingModelActivity()
  expect(
    reduceActivity(turn, { _tag: "StartCommand", command: "reload" }),
  ).toBe(turn)

  const login = commandActivity("login")
  expect(
    reduceActivity(login, { _tag: "FinishCommand", command: "model" }),
  ).toBe(login)
  expect(reduceActivity(login, { _tag: "RequestStop" })).toBe(login)

  expect(reduceActivity(turn, { _tag: "RequestStop" })).toEqual({
    _tag: "Stopping",
    target: "turn",
  })
  expect(
    reduceActivity(
      { _tag: "Compacting", reason: "manual" },
      { _tag: "RequestStop" },
    ),
  ).toEqual({ _tag: "Stopping", target: "compaction" })
})
