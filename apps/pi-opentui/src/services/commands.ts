import type {
  SessionStats,
  SlashCommandInfo,
} from "@earendil-works/pi-coding-agent"

export interface CommandInfo {
  readonly name: string
  readonly description: string
  readonly source: "builtin" | "extension" | "prompt" | "skill"
}

export const builtinCommands: ReadonlyArray<CommandInfo> = [
  {
    name: "help",
    description: "List commands available in this screen",
    source: "builtin",
  },
  {
    name: "session",
    description: "Show session info and stats",
    source: "builtin",
  },
  {
    name: "compact",
    description: "Compact the current context",
    source: "builtin",
  },
  {
    name: "model",
    description: "Choose the model",
    source: "builtin",
  },
  {
    name: "thinking",
    description: "Choose the thinking level",
    source: "builtin",
  },
  {
    name: "new",
    description: "Start a new saved session",
    source: "builtin",
  },
  {
    name: "resume",
    description: "Resume a saved session",
    source: "builtin",
  },
]

export const commandName = (text: string): string | undefined => {
  if (!text.startsWith("/")) return undefined
  const name = text.slice(1).split(/\s/, 1)[0]
  return name === undefined || name.length === 0 ? undefined : name
}

export const commandCatalog = (
  sdkCommands: ReadonlyArray<SlashCommandInfo>,
): ReadonlyArray<CommandInfo> => {
  const commands = new Map<string, CommandInfo>(
    sdkCommands.map((command) => [
      command.name,
      {
        name: command.name,
        description: command.description ?? "",
        source: command.source,
      },
    ]),
  )
  for (const command of builtinCommands) {
    commands.set(command.name, command)
  }
  return [...commands.values()].toSorted((left, right) =>
    left.name.localeCompare(right.name),
  )
}

export const commandHelp = (
  commands: ReadonlyArray<CommandInfo>,
): string =>
  commands
    .map(
      (command) =>
        `/${command.name} — ${command.description}`,
    )
    .join("\n")

export const sessionStatsText = (stats: SessionStats): string =>
  [
    `Session ${stats.sessionId}`,
    stats.sessionFile ?? "Not saved",
    `${stats.userMessages} user · ${stats.assistantMessages} assistant · ${stats.toolCalls} tool calls`,
    `${stats.tokens.total} tokens · $${stats.cost.toFixed(4)}`,
  ].join("\n")
