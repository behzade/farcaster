import type {
  SessionStats,
  SlashCommandInfo,
} from "@earendil-works/pi-coding-agent"

export interface CommandInfo {
  readonly name: string
  readonly description: string
  readonly source: "builtin" | "extension" | "prompt" | "skill"
}

export const builtinCommands = [
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
    name: "login",
    description: "Save provider login or API key",
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
  {
    name: "reload",
    description: "Reload Pi resources and keybindings",
    source: "builtin",
  },
] as const satisfies ReadonlyArray<CommandInfo>

export type BuiltinCommandName =
  (typeof builtinCommands)[number]["name"]

export interface CommandSelection {
  readonly name: string
  readonly arguments: string
}

export const selectSlashCommand = (
  commands: ReadonlyArray<CommandInfo>,
  text: string,
): CommandSelection | undefined => {
  if (!text.startsWith("/")) return undefined
  const separator = text.search(/\s/)
  const end = separator < 0 ? text.length : separator
  const inputName = text.slice(1, end)
  if (inputName.length === 0) return undefined
  const command = commands.find(
    (candidate) =>
      candidate.name.toLowerCase() === inputName.toLowerCase(),
  )
  return command === undefined
    ? undefined
    : {
        name: command.name,
        arguments: separator < 0 ? "" : text.slice(separator).trim(),
      }
}

const defaultCommandMatchLimit = 8

export const exactSlashCommand = (
  commands: ReadonlyArray<CommandInfo>,
  draft: string,
): CommandInfo | undefined => {
  if (!draft.startsWith("/") || /\s/.test(draft.slice(1))) {
    return undefined
  }
  const name = draft.slice(1).toLowerCase()
  if (name.length === 0) return undefined
  return commands.find(
    (command) => command.name.toLowerCase() === name,
  )
}

export const slashCommandMatches = (
  commands: ReadonlyArray<CommandInfo>,
  draft: string,
  limit = defaultCommandMatchLimit,
): ReadonlyArray<CommandInfo> => {
  if (!draft.startsWith("/") || /\s/.test(draft.slice(1))) {
    return []
  }

  const query = draft.slice(1).toLowerCase()
  const maximum = Math.max(0, Math.floor(limit))
  if (maximum === 0) return []

  return commands
    .map((command, index) => {
      const name = command.name.toLowerCase()
      const description = command.description.toLowerCase()
      const rank =
        name === query
          ? 0
          : name.startsWith(query)
            ? 1
            : description.includes(query)
              ? 2
              : 3
      return { command, index, rank }
    })
    .filter((match) => match.rank < 3)
    .toSorted(
      (left, right) =>
        left.rank - right.rank ||
        left.command.name.localeCompare(right.command.name) ||
        left.index - right.index,
    )
    .slice(0, maximum)
    .map((match) => match.command)
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
