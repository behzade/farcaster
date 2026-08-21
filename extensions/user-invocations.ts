import { readFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { ExtensionAPI, SlashCommandInfo } from "@earendil-works/pi-coding-agent";
import type { AutocompleteItem, AutocompleteProvider } from "@earendil-works/pi-tui";

type InvocableCommand = SlashCommandInfo & { invocationName: string };

export type InvocationStack = {
  commands: InvocableCommand[];
  displayText: string;
  argumentsText: string;
};

function displayName(command: SlashCommandInfo): string {
  return command.source === "skill" ? command.name.replace(/^skill:/, "") : command.name;
}

export function invocableCommands(commands: SlashCommandInfo[]): InvocableCommand[] {
  const seen = new Set<string>();
  const candidates = commands
    .filter((command) => {
      if (command.source !== "prompt" && command.source !== "skill") return false;
      const key = `${command.source}:${command.name}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .map((command) => ({ ...command, invocationName: displayName(command) }));
  const counts = new Map<string, number>();
  for (const command of candidates) {
    counts.set(command.invocationName, (counts.get(command.invocationName) ?? 0) + 1);
  }
  return candidates.map((command) => ({
    ...command,
    invocationName:
      counts.get(command.invocationName) === 1
        ? command.invocationName
        : `${command.source}:${command.invocationName}`,
  }));
}

export function parseInvocationStack(text: string, commands: SlashCommandInfo[]): InvocationStack | undefined {
  const byName = new Map(invocableCommands(commands).map((command) => [command.invocationName, command]));
  const selected: InvocableCommand[] = [];
  const arguments_: string[] = [];
  const invocation = /(^|\s)\$([a-z0-9][a-z0-9:_-]{0,127})(?=\s|$)/g;
  let cursor = 0;

  for (const match of text.matchAll(invocation)) {
    const name = match[2] ?? "";
    const command = byName.get(name);
    if (!command) continue;
    const start = (match.index ?? 0) + (match[1]?.length ?? 0);
    const argument = text.slice(cursor, start).trim();
    if (argument) arguments_.push(argument);
    selected.push(command);
    cursor = start + name.length + 1;
  }
  if (selected.length === 0) return undefined;
  const trailing = text.slice(cursor).trim();
  if (trailing) arguments_.push(trailing);

  return {
    commands: selected,
    displayText: text,
    argumentsText: arguments_.join(" "),
  };
}

function invocationQuery(beforeCursor: string): string | undefined {
  return beforeCursor.match(/(?:^|[ \t])\$([a-z0-9:_-]*)$/)?.[1];
}

function invocationItem(command: InvocableCommand): AutocompleteItem {
  const kind = command.source === "skill" ? "Skill" : "Prompt";
  return {
    value: `$${command.invocationName}`,
    label: `$${command.invocationName}`,
    description: command.description ? `${kind} · ${command.description}` : kind,
  };
}

function createAutocompleteProvider(pi: ExtensionAPI, current: AutocompleteProvider): AutocompleteProvider {
  return {
    triggerCharacters: ["$"],
    async getSuggestions(lines, cursorLine, cursorCol, options) {
      const beforeCursor = (lines[cursorLine] ?? "").slice(0, cursorCol);
      const commands = pi.getCommands();
      const query = invocationQuery(beforeCursor);
      if (query === undefined) {
        const suggestions = await current.getSuggestions(lines, cursorLine, cursorCol, options);
        if (!suggestions || !beforeCursor.trimStart().startsWith("/") || beforeCursor.includes(" ")) {
          return suggestions;
        }
        const hidden = new Set(
          commands
            .filter((command) => command.source === "prompt" || command.source === "skill")
            .map((command) => command.name),
        );
        const items = suggestions.items.filter((item) => !hidden.has(item.value));
        return items.length > 0 ? { ...suggestions, items } : null;
      }
      const matches = invocableCommands(commands).filter((command) =>
        command.invocationName.includes(query),
      );
      if (matches.length === 0) return null;
      return { prefix: `$${query}`, items: matches.map(invocationItem) };
    },
    applyCompletion(lines, cursorLine, cursorCol, item, prefix) {
      if (!prefix.startsWith("$") || !item.value.startsWith("$")) {
        return current.applyCompletion(lines, cursorLine, cursorCol, item, prefix);
      }
      const line = lines[cursorLine] ?? "";
      const before = line.slice(0, cursorCol - prefix.length);
      const after = line.slice(cursorCol).replace(/^ /, "");
      const replacement = `${item.value} `;
      const next = [...lines];
      next[cursorLine] = `${before}${replacement}${after}`;
      return { lines: next, cursorLine, cursorCol: before.length + replacement.length };
    },
    shouldTriggerFileCompletion(lines, cursorLine, cursorCol) {
      const beforeCursor = (lines[cursorLine] ?? "").slice(0, cursorCol);
      if (invocationQuery(beforeCursor) !== undefined) return true;
      return current.shouldTriggerFileCompletion?.(lines, cursorLine, cursorCol) ?? true;
    },
  };
}

function stripFrontmatter(markdown: string): string {
  return markdown.replace(/^---\r?\n[\s\S]*?\r?\n---(?:\r?\n|$)/, "").trim();
}

function parseArguments(text: string): string[] {
  const arguments_: string[] = [];
  let current = "";
  let quote: string | undefined;
  for (const character of text) {
    if (quote) {
      if (character === quote) quote = undefined;
      else current += character;
    } else if (character === '"' || character === "'") {
      quote = character;
    } else if (/\s/.test(character)) {
      if (current) arguments_.push(current);
      current = "";
    } else {
      current += character;
    }
  }
  if (current) arguments_.push(current);
  return arguments_;
}

function substituteArguments(content: string, arguments_: string[]): string {
  const all = arguments_.join(" ");
  return content.replace(
    /\$\{(\d+|ARGUMENTS|@):-([^}]*)\}|\$\{@:(\d+)(?::(\d+))?\}|\$(ARGUMENTS|@|\d+)/g,
    (_match, defaultTarget, defaultValue, sliceStart, sliceLength, simple) => {
      if (defaultTarget) {
        const value = defaultTarget === "@" || defaultTarget === "ARGUMENTS"
          ? all
          : arguments_[Number(defaultTarget) - 1];
        return value || defaultValue;
      }
      if (sliceStart) {
        const start = Math.max(0, Number(sliceStart) - 1);
        return arguments_.slice(start, sliceLength ? start + Number(sliceLength) : undefined).join(" ");
      }
      if (simple === "@" || simple === "ARGUMENTS") return all;
      return arguments_[Number(simple) - 1] ?? "";
    },
  );
}

async function expandCommand(
  command: InvocableCommand,
  argumentsText: string,
): Promise<{ text: string; usedArguments: boolean }> {
  const raw = await readFile(command.sourceInfo.path, "utf8");
  const body = stripFrontmatter(raw);
  if (command.source === "prompt") {
    const text = substituteArguments(body, parseArguments(argumentsText));
    return { text, usedArguments: text !== body };
  }
  const skillName = displayName(command);
  const baseDir = command.sourceInfo.baseDir ?? dirname(command.sourceInfo.path);
  return {
    text: `<skill name="${skillName}" location="${command.sourceInfo.path}">\nReferences are relative to ${baseDir}.\n\n${body}\n</skill>`,
    usedArguments: false,
  };
}

export async function expandInvocationStack(stack: InvocationStack): Promise<string> {
  const expanded = await Promise.all(
    stack.commands.map((command) => expandCommand(command, stack.argumentsText)),
  );
  const appendArguments = stack.commands.some((command) => command.source === "skill")
    || expanded.every((result) => !result.usedArguments);
  const parts = expanded.map((result) => result.text);
  if (stack.argumentsText && appendArguments) parts.push(stack.argumentsText);
  return parts.join("\n\n");
}

export default function userInvocations(pi: ExtensionAPI): void {
  const pendingDisplays = new Map<string, string[]>();
  pi.on("session_start", (_event, ctx) => {
    if (ctx.mode === "tui") {
      ctx.ui.addAutocompleteProvider((current) => createAutocompleteProvider(pi, current));
    }
  });

  pi.on("input", async (event, ctx) => {
    if (event.source === "extension" || !event.text) return { action: "continue" };
    if (event.text.startsWith("\\$")) {
      return { action: "transform", text: event.text.slice(1) };
    }
    const stack = parseInvocationStack(event.text, pi.getCommands());
    if (!stack) return { action: "continue" };
    try {
      const text = await expandInvocationStack(stack);
      const displays = pendingDisplays.get(text) ?? [];
      displays.push(stack.displayText);
      pendingDisplays.set(text, displays);
      return { action: "transform", text };
    } catch (error) {
      ctx.ui.notify(
        `Could not expand ${stack.commands.map((command) => `$${command.invocationName}`).join(" ")}: ${error instanceof Error ? error.message : String(error)}`,
        "error",
      );
      return { action: "handled" };
    }
  });

  pi.on("message_end", (event) => {
    if (event.message.role !== "user") return;
    const text = event.message.content
      .filter((content) => content.type === "text")
      .map((content) => content.text)
      .join("\n");
    const displays = pendingDisplays.get(text);
    const displayText = displays?.shift();
    if (!displayText) return;
    if (displays.length === 0) pendingDisplays.delete(text);
    return {
      message: { ...event.message, piUserInvocation: displayText } as typeof event.message,
    };
  });
}
