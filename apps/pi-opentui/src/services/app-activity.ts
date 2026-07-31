import type { AgentSessionEvent } from "@earendil-works/pi-coding-agent"

type CompactionStart = Extract<
  AgentSessionEvent,
  { readonly type: "compaction_start" }
>

export type AppCommandActivity =
  | "compact"
  | "login"
  | "model"
  | "new-session"
  | "reload"
  | "resume"
  | "thinking"

export type TurnStage =
  | { readonly _tag: "AwaitingModel" }
  | { readonly _tag: "Streaming" }
  | { readonly _tag: "RunningTools" }
  | {
      readonly _tag: "Retrying"
      readonly attempt: number
      readonly maxAttempts: number
      readonly delayMs: number
    }

export type AppActivity =
  | { readonly _tag: "Idle" }
  | {
      readonly _tag: "Command"
      readonly command: AppCommandActivity
    }
  | {
      readonly _tag: "Turn"
      readonly stage: TurnStage
    }
  | {
      readonly _tag: "Compacting"
      readonly reason: CompactionStart["reason"]
    }
  | {
      readonly _tag: "Stopping"
      readonly target: "turn" | "compaction"
    }
  | {
      readonly _tag: "Failed"
      readonly message: string
    }
  | {
      readonly _tag: "Fatal"
      readonly message: string
    }

export type AppActivityAction =
  | { readonly _tag: "StartTurn" }
  | { readonly _tag: "ContinueAfterCompaction" }
  | { readonly _tag: "StartCommand"; readonly command: AppCommandActivity }
  | { readonly _tag: "FinishCommand"; readonly command: AppCommandActivity }
  | { readonly _tag: "RequestStop" }
  | { readonly _tag: "TurnResolved" }
  | { readonly _tag: "Fail"; readonly message: string }
  | { readonly _tag: "Fatal"; readonly message: string }
  | { readonly _tag: "SessionEvent"; readonly event: AgentSessionEvent }

export const idleActivity: AppActivity = { _tag: "Idle" }

export const awaitingModelActivity = (): AppActivity => ({
  _tag: "Turn",
  stage: { _tag: "AwaitingModel" },
})

export const commandActivity = (
  command: AppCommandActivity,
): AppActivity => ({ _tag: "Command", command })

export const failedActivity = (message: string): AppActivity => ({
  _tag: "Failed",
  message,
})

export const fatalActivity = (message: string): AppActivity => ({
  _tag: "Fatal",
  message,
})

export const activityError = (
  activity: AppActivity,
): string | undefined =>
  activity._tag === "Failed" || activity._tag === "Fatal"
    ? activity.message
    : undefined

export const activityStatus = (activity: AppActivity): string => {
  switch (activity._tag) {
    case "Idle":
      return "ready"
    case "Command":
      return activity.command
    case "Turn":
      switch (activity.stage._tag) {
        case "AwaitingModel":
          return "waiting"
        case "Streaming":
          return "streaming"
        case "RunningTools":
          return "tools"
        case "Retrying":
          return `retry ${activity.stage.attempt}/${activity.stage.maxAttempts}`
      }
    case "Compacting":
      return "compacting"
    case "Stopping":
      return "stopping"
    case "Failed":
      return "error"
    case "Fatal":
      return "fatal"
  }
}

export const canStartPrompt = (activity: AppActivity): boolean =>
  activity._tag === "Idle" || activity._tag === "Failed"

export const canAcceptInput = (activity: AppActivity): boolean =>
  activity._tag === "Idle" ||
  activity._tag === "Failed" ||
  activity._tag === "Turn" ||
  activity._tag === "Compacting"

export const canInterrupt = (activity: AppActivity): boolean =>
  activity._tag === "Turn" ||
  activity._tag === "Compacting" ||
  (activity._tag === "Command" && activity.command === "login")

export type PromptRoute = "start" | "steer" | "after-compaction" | "reject"

export const promptRoute = (activity: AppActivity): PromptRoute => {
  switch (activity._tag) {
    case "Idle":
    case "Failed":
      return "start"
    case "Turn":
      return "steer"
    case "Compacting":
      return "after-compaction"
    case "Command":
    case "Stopping":
    case "Fatal":
      return "reject"
  }
}

export const isStoppingTurn = (activity: AppActivity): boolean =>
  activity._tag === "Stopping" && activity.target === "turn"

export const showsAgentWorking = (activity: AppActivity): boolean =>
  activity._tag === "Turn" ||
  activity._tag === "Compacting" ||
  activity._tag === "Stopping"

const messageRole = (message: unknown): unknown =>
  typeof message === "object" && message !== null && "role" in message
    ? message.role
    : undefined

const acceptsTurnEvents = (activity: AppActivity): boolean =>
  activity._tag === "Idle" ||
  activity._tag === "Failed" ||
  activity._tag === "Turn"

const reduceActivityEvent = (
  activity: AppActivity,
  event: AgentSessionEvent,
): AppActivity => {
  if (activity._tag === "Fatal") return activity
  if (
    activity._tag === "Stopping" &&
    event.type !== "agent_settled" &&
    event.type !== "compaction_end"
  ) {
    return activity
  }

  switch (event.type) {
    case "agent_start":
    case "turn_start":
      return acceptsTurnEvents(activity) ? awaitingModelActivity() : activity

    case "message_start":
    case "message_update":
      return messageRole(event.message) === "assistant" &&
        acceptsTurnEvents(activity)
        ? { _tag: "Turn", stage: { _tag: "Streaming" } }
        : activity

    case "tool_execution_start":
    case "tool_execution_update":
    case "tool_execution_end":
      return acceptsTurnEvents(activity)
        ? { _tag: "Turn", stage: { _tag: "RunningTools" } }
        : activity

    case "compaction_start":
      return acceptsTurnEvents(activity) ||
        (activity._tag === "Command" && activity.command === "compact")
        ? { _tag: "Compacting", reason: event.reason }
        : activity

    case "compaction_end":
      if (activity._tag === "Turn") return activity
      if (
        activity._tag !== "Compacting" &&
        !(
          activity._tag === "Stopping" &&
          activity.target === "compaction"
        )
      ) {
        return activity
      }
      if (event.errorMessage !== undefined) {
        return failedActivity(event.errorMessage)
      }
      return event.willRetry ? awaitingModelActivity() : idleActivity

    case "auto_retry_start":
      return acceptsTurnEvents(activity)
        ? {
            _tag: "Turn",
            stage: {
              _tag: "Retrying",
              attempt: event.attempt,
              maxAttempts: event.maxAttempts,
              delayMs: event.delayMs,
            },
          }
        : activity

    case "auto_retry_end":
      return acceptsTurnEvents(activity)
        ? event.success
          ? awaitingModelActivity()
          : failedActivity(event.finalError ?? `Retry ${event.attempt} failed`)
        : activity

    case "agent_settled":
      return activity._tag === "Turn" ||
        (activity._tag === "Stopping" && activity.target === "turn")
        ? idleActivity
        : activity

    case "agent_end":
    case "turn_end":
    case "message_end":
    case "queue_update":
    case "entry_appended":
    case "session_info_changed":
    case "thinking_level_changed":
      return activity
  }
}

export const reduceActivity = (
  activity: AppActivity,
  action: AppActivityAction,
): AppActivity => {
  if (activity._tag === "Fatal") return activity

  switch (action._tag) {
    case "StartTurn":
      return canStartPrompt(activity) ? awaitingModelActivity() : activity
    case "ContinueAfterCompaction":
      return activity._tag === "Compacting"
        ? awaitingModelActivity()
        : activity
    case "StartCommand":
      return canStartPrompt(activity)
        ? commandActivity(action.command)
        : activity
    case "FinishCommand":
      return activity._tag === "Command" &&
        activity.command === action.command
        ? idleActivity
        : activity
    case "RequestStop":
      return activity._tag === "Turn"
        ? { _tag: "Stopping", target: "turn" }
        : activity._tag === "Compacting"
          ? { _tag: "Stopping", target: "compaction" }
          : activity
    case "TurnResolved":
      return activity._tag === "Turn" ||
        (activity._tag === "Stopping" && activity.target === "turn")
        ? idleActivity
        : activity
    case "Fail":
      return failedActivity(action.message)
    case "Fatal":
      return fatalActivity(action.message)
    case "SessionEvent":
      return reduceActivityEvent(activity, action.event)
  }
}
