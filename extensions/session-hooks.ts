import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { getAgentDir } from "@earendil-works/pi-coding-agent";

interface HookConfig {
  afterAgent: string[];
}

interface SessionData {
  version: 1;
  type: "agent-turn-complete";
  sessionId: string;
  turnId: string;
  cwd: string;
  sessionFile?: string;
  inputMessages: string[];
  lastAssistantMessage?: string;
  timestamp: string;
}

function loadConfig(): HookConfig {
  const path = join(getAgentDir(), "extensions", "hooks.json");
  if (!existsSync(path)) return { afterAgent: [] };
  try {
    return { afterAgent: [], ...(JSON.parse(readFileSync(path, "utf8")) as Partial<HookConfig>) };
  } catch (error) {
    console.error(`Could not load hook settings ${path}: ${error}`);
    return { afterAgent: [] };
  }
}

function sessionSummary(ctx: ExtensionContext, turnId: string): string {
  return `Session ${ctx.sessionManager.getSessionId().slice(0, 8)} · turn ${turnId.split(":").at(-1)}`;
}

export default function (pi: ExtensionAPI) {
  const config = loadConfig();
  let turnId = "pending:0";
  let inputMessages: string[] = [];
  let lastAssistantMessage: string | undefined;

  pi.on("session_start", (_event, ctx) => {
    const id = ctx.sessionManager.getSessionId();
    turnId = `${id}:0`;
    ctx.ui.setStatus("session-id", sessionSummary(ctx, turnId));
  });

  pi.on("before_agent_start", (event) => {
    inputMessages = [event.prompt];
    lastAssistantMessage = undefined;
  });

  pi.on("turn_start", (event, ctx) => {
    turnId = `${ctx.sessionManager.getSessionId()}:${event.turnIndex}`;
    ctx.ui.setStatus("session-id", sessionSummary(ctx, turnId));
  });

  pi.on("message_end", (event) => {
    if (event.message.role !== "assistant") return;
    const text = event.message.content.filter((part) => part.type === "text").map((part) => part.text).join("\n").trim();
    if (text) lastAssistantMessage = text;
  });

  pi.on("agent_settled", (_event, ctx) => {
    if (ctx.hasPendingMessages()) return;
    const payload: SessionData = {
      version: 1,
      type: "agent-turn-complete",
      sessionId: ctx.sessionManager.getSessionId(),
      turnId,
      cwd: ctx.cwd,
      sessionFile: ctx.sessionManager.getSessionFile(),
      inputMessages,
      lastAssistantMessage,
      timestamp: new Date().toISOString(),
    };
    pi.events.emit("hook:after-agent", payload);

    const [command, ...args] = config.afterAgent;
    if (command) {
      const child = spawn(command, [...args, JSON.stringify(payload)], { detached: true, stdio: "ignore" });
      child.unref();
    }
  });

  pi.registerCommand("session-info", {
    description: "Show stable session and turn IDs",
    handler: (_args, ctx) => {
      ctx.ui.notify(`${sessionSummary(ctx, turnId)}\n${ctx.sessionManager.getSessionFile() ?? "Session is not persisted"}`, "info");
    },
  });
}
