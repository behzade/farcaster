import type {
  AgentSessionEvent,
  SessionStats,
} from "@earendil-works/pi-coding-agent"
import type { Effect } from "effect"
import type { AppActivity } from "./app-activity.ts"
import type { CommandInfo } from "./commands.ts"
import type { AppDialog } from "./extension-ui.ts"
import type { LiveUsage } from "./live-usage.ts"
import type {
  ExtensionLoadError,
  PiModelInfo,
  PiThinkingLevel,
  PromptDelivery,
  PromptQueue,
} from "./pi-session.ts"
import type { TranscriptModel } from "./transcript.ts"

export type { PromptDelivery, PromptQueue } from "./pi-session.ts"
export type { AppActivity } from "./app-activity.ts"

export interface DraftRestore {
  readonly id: number
  readonly text: string
}

export interface AppSnapshot {
  readonly cwd: string
  readonly hideThinkingBlock: boolean
  readonly activity: AppActivity
  readonly activeTools: ReadonlyArray<string>
  readonly model: PiModelInfo | undefined
  readonly thinkingLevel: PiThinkingLevel
  readonly sessionStats: SessionStats
  readonly liveUsage: LiveUsage
  readonly extensionPaths: ReadonlyArray<string>
  readonly extensionErrors: ReadonlyArray<ExtensionLoadError>
  readonly eventCount: number
  readonly lastEvent: AgentSessionEvent["type"] | undefined
  readonly transcript: TranscriptModel
  readonly dialog: AppDialog | undefined
  readonly authNotice: string | undefined
  readonly statuses: Readonly<Record<string, string>>
  readonly commands: ReadonlyArray<CommandInfo>
  readonly promptQueue: PromptQueue
  readonly draftRestore: DraftRestore | undefined
}

export type AppCommand =
  | {
      readonly _tag: "Prompt"
      readonly text: string
      readonly delivery: PromptDelivery
    }
  | {
      readonly _tag: "RunCommand"
      readonly name: string
      readonly arguments: string
      readonly delivery: PromptDelivery
    }
  | { readonly _tag: "Abort" }
  | { readonly _tag: "Dequeue" }
  | { readonly _tag: "AcknowledgeDraftRestore"; readonly id: number }
  | {
      readonly _tag: "ResolveDialog"
      readonly id: number
      readonly value: string | undefined
    }

export type UpdateAppState = (
  update: (snapshot: AppSnapshot) => AppSnapshot,
) => Effect.Effect<void>

export type PushAppNotice = (
  message: string,
  isError?: boolean,
) => Effect.Effect<void>

export const applyAppStateUpdate = (
  snapshot: AppSnapshot,
  update: (current: AppSnapshot) => AppSnapshot,
): AppSnapshot => snapshot.activity._tag === "Fatal" ? snapshot : update(snapshot)
