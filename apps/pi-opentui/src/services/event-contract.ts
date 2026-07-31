import type { AgentSessionEvent } from "@earendil-works/pi-coding-agent"

const unknownEventType = (event: unknown): string => {
  if (typeof event !== "object" || event === null || !("type" in event)) {
    return String(event)
  }
  return String(event.type)
}

const unhandledEvent = (event: never): never => {
  throw new Error(
    `Unhandled AgentSessionEvent type: ${unknownEventType(event)}`,
  )
}

/**
 * Keep this switch in step with Pi's AgentSessionEvent union.
 *
 * The never call makes a new Pi event a type error. The same branch rejects
 * untyped or version-skewed events at runtime, where TypeScript cannot help.
 */
export const assertAgentSessionEventContract = (
  event: AgentSessionEvent,
): void => {
  switch (event.type) {
    case "agent_start":
    case "agent_end":
    case "agent_settled":
    case "turn_start":
    case "turn_end":
    case "message_start":
    case "message_update":
    case "message_end":
    case "tool_execution_start":
    case "tool_execution_update":
    case "tool_execution_end":
    case "queue_update":
    case "compaction_start":
    case "compaction_end":
    case "entry_appended":
    case "session_info_changed":
    case "thinking_level_changed":
    case "auto_retry_start":
    case "auto_retry_end":
      return
    default:
      return unhandledEvent(event)
  }
}
