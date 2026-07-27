import { setComposerMargin } from "./chat-layout.ts";

import type {
  AgentToolResult,
  ExtensionAPI,
  ReadToolInput,
  Theme,
} from "@earendil-works/pi-coding-agent";
import {
  createFindTool,
  createGrepTool,
  createLsTool,
  createReadTool,
  createWriteTool,
} from "@earendil-works/pi-coding-agent";
import { Container, Text } from "@earendil-works/pi-tui";
import { homedir } from "node:os";

interface ReadEntry {
  id: string;
  args: ReadToolInput;
  result?: AgentToolResult<unknown>;
  partial: boolean;
  isError: boolean;
}

interface ReadGroup {
  entries: ReadEntry[];
  invalidateLeader?: () => void;
}

const readGroupsById = new Map<string, ReadGroup>();
let currentReadGroup: ReadGroup | undefined;

function displayPath(path: string): string {
  const home = homedir();
  return path.startsWith(`${home}/`) ? `~/${path.slice(home.length + 1)}` : path;
}

function ensureReadEntry(id: string, args: ReadToolInput): { group: ReadGroup; entry: ReadEntry } {
  const knownGroup = readGroupsById.get(id);
  if (knownGroup) {
    const entry = knownGroup.entries.find((candidate) => candidate.id === id)!;
    entry.args = args;
    return { group: knownGroup, entry };
  }

  const group = currentReadGroup ?? { entries: [] };
  currentReadGroup = group;
  const entry: ReadEntry = { id, args, partial: true, isError: false };
  group.entries.push(entry);
  readGroupsById.set(id, group);
  return { group, entry };
}

function moveReadEntry(entry: ReadEntry, from: ReadGroup, to: ReadGroup): void {
  from.entries = from.entries.filter((candidate) => candidate !== entry);
  to.entries.push(entry);
  readGroupsById.set(entry.id, to);
  from.invalidateLeader?.();
}

function readRange(args: ReadToolInput): string {
  if (args.offset === undefined && args.limit === undefined) return "";
  const start = args.offset ?? 1;
  const end = args.limit === undefined ? "" : `-${start + args.limit - 1}`;
  return `:${start}${end}`;
}

function textResult(result: AgentToolResult<unknown> | undefined): string | undefined {
  const block = result?.content.find((item) => item.type === "text");
  return block?.type === "text" ? block.text : undefined;
}

function resultLabel(entry: ReadEntry): string {
  if (entry.partial || !entry.result) return "…";
  if (entry.isError) return "✗";
  if (entry.result.content.some((item) => item.type === "image")) return "image";
  const text = textResult(entry.result);
  if (text === undefined) return "✓";
  return `${text.split("\n").length} lines`;
}

function renderReadGroup(group: ReadGroup, expanded: boolean, theme: Theme): string {
  if (group.entries.length === 1) {
    const entry = group.entries[0]!;
    let output = theme.fg("toolTitle", theme.bold("read "));
    output += theme.fg("accent", `${displayPath(entry.args.path)}${readRange(entry.args)}`);
    output += theme.fg(entry.isError ? "error" : "dim", `  ${resultLabel(entry)}`);
    if (expanded) {
      const text = textResult(entry.result);
      if (text) output += `\n${theme.fg("toolOutput", text)}`;
    }
    return output;
  }

  const done = group.entries.filter((entry) => !entry.partial && !entry.isError).length;
  const failed = group.entries.filter((entry) => entry.isError).length;
  let output = theme.fg("toolTitle", theme.bold(`read ${group.entries.length} files`));
  output += theme.fg(failed ? "error" : "dim", `  ${done}/${group.entries.length}${failed ? `, ${failed} failed` : ""}`);

  for (const entry of group.entries) {
    const color = entry.isError ? "error" : entry.partial ? "warning" : "success";
    output += `\n${theme.fg(color, entry.isError ? "✗" : entry.partial ? "…" : "✓")}`;
    output += ` ${theme.fg("accent", `${displayPath(entry.args.path)}${readRange(entry.args)}`)}`;
    output += theme.fg("dim", `  ${resultLabel(entry)}`);
    if (expanded) {
      const text = textResult(entry.result);
      if (text) output += `\n${theme.fg("toolOutput", text)}`;
    }
  }
  return output;
}

export default function denseTools(pi: ExtensionAPI) {
  const cwd = process.cwd();
  const read = createReadTool(cwd);
  const tools = [
    createWriteTool(cwd),
    createFindTool(cwd),
    createGrepTool(cwd),
    createLsTool(cwd),
  ];

  for (const tool of tools) {
    pi.registerTool({ ...tool, renderShell: "self" });
  }

  pi.registerTool({
    ...read,
    renderShell: "self",
    renderCall(args, theme, context) {
      const { group, entry } = ensureReadEntry(context.toolCallId, args);
      if (group.entries[0] !== entry) return new Container();
      group.invalidateLeader = context.invalidate;
      const text = context.lastComponent instanceof Text ? context.lastComponent : new Text("", 0, 0);
      text.setText(renderReadGroup(group, context.expanded, theme));
      return text;
    },
    renderResult(result, options, _theme, context) {
      const { group, entry } = ensureReadEntry(context.toolCallId, context.args);
      const changed =
        entry.result?.content !== result.content ||
        entry.result?.details !== result.details ||
        entry.partial !== options.isPartial ||
        entry.isError !== context.isError;
      entry.result = result;
      entry.partial = options.isPartial;
      entry.isError = context.isError;
      if (changed) queueMicrotask(() => group.invalidateLeader?.());
      return new Container();
    },
  });

  pi.on("turn_start", () => {
    currentReadGroup = undefined;
  });

  pi.on("tool_execution_start", (event) => {
    if (event.toolName !== "read") {
      currentReadGroup = undefined;
      return;
    }

    const { group } = ensureReadEntry(event.toolCallId, event.args as ReadToolInput);
    currentReadGroup = group;
    group.invalidateLeader?.();
  });

  pi.on("turn_end", () => {
    currentReadGroup = undefined;
  });

  pi.on("message_end", (event) => {
    if (event.message.role !== "assistant") return;
    const claimedGroups = new Set<ReadGroup>();
    let desiredGroup: ReadGroup | undefined;

    for (const content of event.message.content) {
      if (content.type !== "toolCall" || content.name !== "read") {
        desiredGroup = undefined;
        continue;
      }

      const knownGroup = readGroupsById.get(content.id);
      const knownEntry = knownGroup?.entries.find((candidate) => candidate.id === content.id);
      if (!desiredGroup) {
        desiredGroup = knownGroup && !claimedGroups.has(knownGroup) ? knownGroup : { entries: [] };
        claimedGroups.add(desiredGroup);
      }

      if (knownGroup && knownEntry && knownGroup !== desiredGroup) {
        moveReadEntry(knownEntry, knownGroup, desiredGroup);
      } else if (!knownEntry) {
        const entry: ReadEntry = {
          id: content.id,
          args: content.arguments as ReadToolInput,
          partial: true,
          isError: false,
        };
        desiredGroup.entries.push(entry);
        readGroupsById.set(entry.id, desiredGroup);
      }
    }
    currentReadGroup = undefined;
  });

  pi.on("session_start", (_event, ctx) => {
    setComposerMargin(ctx);
    readGroupsById.clear();
    currentReadGroup = undefined;

    for (const sessionEntry of ctx.sessionManager.getBranch()) {
      if (sessionEntry.type !== "message" || sessionEntry.message.role !== "assistant") continue;
      currentReadGroup = undefined;
      for (const content of sessionEntry.message.content) {
        if (content.type === "toolCall" && content.name === "read") {
          ensureReadEntry(content.id, content.arguments as ReadToolInput);
        } else {
          currentReadGroup = undefined;
        }
      }
      currentReadGroup = undefined;
    }
  });

  pi.on("session_shutdown", () => {
    readGroupsById.clear();
    currentReadGroup = undefined;
  });
}
