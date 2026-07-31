import type { SessionStats } from "@earendil-works/pi-coding-agent"
import type { Effect } from "effect"
import type { CommandInfo } from "./commands.ts"
import type { AppDialog } from "./extension-ui.ts"
import type { LiveUsage } from "./live-usage.ts"
import type {
  ExtensionLoadError,
  PiModelInfo,
  PiThinkingLevel,
} from "./pi-session.ts"
import type { TranscriptModel } from "./transcript.ts"

export type AppPhase = "ready" | "running" | "stopping" | "error" | "fatal"

export interface AppSnapshot {
  readonly cwd: string
  readonly hideThinkingBlock: boolean
  readonly phase: AppPhase
  readonly activeTools: ReadonlyArray<string>
  readonly model: PiModelInfo | undefined
  readonly thinkingLevel: PiThinkingLevel
  readonly sessionStats: SessionStats
  readonly liveUsage: LiveUsage
  readonly extensionPaths: ReadonlyArray<string>
  readonly extensionErrors: ReadonlyArray<ExtensionLoadError>
  readonly eventCount: number
  readonly lastEvent: string | undefined
  readonly error: string | undefined
  readonly transcript: TranscriptModel
  readonly dialog: AppDialog | undefined
  readonly authNotice: string | undefined
  readonly statuses: Readonly<Record<string, string>>
  readonly commands: ReadonlyArray<CommandInfo>
}

export type AppCommand =
  | { readonly _tag: "Prompt"; readonly text: string }
  | { readonly _tag: "Abort" }
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
): AppSnapshot => snapshot.phase === "fatal" ? snapshot : update(snapshot)
