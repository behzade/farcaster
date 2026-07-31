import { expect, test } from "bun:test"
import {
  exactSlashCommand,
  selectSlashCommand,
  slashCommandMatches,
  type CommandInfo,
} from "../src/services/commands.ts"

const commands: ReadonlyArray<CommandInfo> = [
  {
    name: "resume",
    description: "Resume a saved session",
    source: "builtin",
  },
  {
    name: "reload",
    description: "Reload extensions",
    source: "extension",
  },
  {
    name: "model",
    description: "Choose the active model",
    source: "builtin",
  },
  {
    name: "provider",
    description: "Choose a model provider",
    source: "extension",
  },
]

test("matches slash command drafts by ranked name and description", () => {
  expect(
    slashCommandMatches(commands, "/re").map((command) => command.name),
  ).toEqual(["reload", "resume"])
  expect(
    slashCommandMatches(commands, "/model").map(
      (command) => command.name,
    ),
  ).toEqual(["model", "provider"])
})

test("only matches command names before arguments begin", () => {
  expect(slashCommandMatches(commands, "model")).toEqual([])
  expect(slashCommandMatches(commands, "/model ")).toEqual([])
  expect(slashCommandMatches(commands, "/model fast")).toEqual([])
  expect(slashCommandMatches(commands, "/model\nfast")).toEqual([])
})

test("caps matches and keeps exact command first", () => {
  expect(
    slashCommandMatches(commands, "/", 2).map(
      (command) => command.name,
    ),
  ).toEqual(["model", "provider"])
  expect(
    slashCommandMatches(commands, "/RESUME").map(
      (command) => command.name,
    ),
  ).toEqual(["resume"])
  expect(slashCommandMatches(commands, "/", 0)).toEqual([])
})

test("uses an eight-item default limit", () => {
  const manyCommands = Array.from({ length: 10 }, (_, index) => ({
    name: `command-${index}`,
    description: "",
    source: "extension" as const,
  }))

  expect(slashCommandMatches(manyCommands, "/")).toHaveLength(8)
})

test("finds an exact command only for a complete command name", () => {
  expect(exactSlashCommand(commands, "/MODEL")).toEqual(commands[2])
  expect(exactSlashCommand(commands, "/mod")).toBeUndefined()
  expect(exactSlashCommand(commands, "/model fast")).toBeUndefined()
  expect(exactSlashCommand(commands, "/")).toBeUndefined()
})

test("selects only names from the shown command list", () => {
  expect(selectSlashCommand(commands, "/MODEL fast")).toEqual({
    name: "model",
    arguments: "fast",
  })
  expect(selectSlashCommand(commands, "/tmp/paste.txt")).toBeUndefined()
  expect(selectSlashCommand(commands, "/missing")).toBeUndefined()
  expect(selectSlashCommand(commands, "explain /model")).toBeUndefined()
})
