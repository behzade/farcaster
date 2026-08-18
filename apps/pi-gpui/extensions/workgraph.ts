import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { truncateHead, DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES } from "@earendil-works/pi-coding-agent";
import { StringEnum } from "@earendil-works/pi-ai";
import { Type, type Static } from "typebox";

const searchSchema = Type.Object({
  view: StringEnum(["status", "issue", "ready", "blocked", "next", "graph", "session"] as const),
  status: Type.Optional(StringEnum(["open", "in_progress", "blocked", "done", "cancelled"] as const)),
  number: Type.Optional(Type.Integer({ minimum: 1 })),
});

const editSchema = Type.Object({
  action: StringEnum(["create", "set_status", "add_note", "add_dependency", "remove_dependency", "link_session", "unlink_session"] as const),
  title: Type.Optional(Type.String()),
  body: Type.Optional(Type.String()),
  priority: Type.Optional(Type.Integer({ minimum: 0 })),
  number: Type.Optional(Type.Integer({ minimum: 1 })),
  status: Type.Optional(StringEnum(["open", "in_progress", "blocked", "done", "cancelled"] as const)),
  dependsOn: Type.Optional(Type.Integer({ minimum: 1 })),
  expectedVersion: Type.Optional(Type.Integer({ minimum: 1 })),
});

type SearchInput = Static<typeof searchSchema>;
type EditInput = Static<typeof editSchema>;

function command(): string {
  const value = process.env.PI_GPUI_WORKGRAPH_COMMAND;
  if (!value) throw new Error("Pi GPUI did not provide its work graph command");
  return value;
}

function fields(input: Record<string, unknown>): string[] {
  return Object.entries(input)
    .filter(([, value]) => value !== undefined)
    .map(([key, value]) => `${key}=${String(value)}`);
}

async function run(
  pi: ExtensionAPI,
  operation: "search" | "edit",
  input: Record<string, unknown>,
  ctx: ExtensionContext,
  signal: AbortSignal | undefined,
): Promise<string> {
  const result = await pi.exec(command(), ["workgraph", operation, `project=${ctx.cwd}`, ...fields(input)], { signal });
  if (result.code !== 0) throw new Error(result.stderr.trim() || `work graph ${operation} failed`);
  const truncated = truncateHead(result.stdout, { maxBytes: DEFAULT_MAX_BYTES, maxLines: DEFAULT_MAX_LINES });
  return truncated.truncated ? `${truncated.content}\n[Work graph output truncated]` : truncated.content;
}

export default function workgraph(pi: ExtensionAPI): void {
  pi.registerTool({
    name: "workgraph_search",
    label: "Work graph search",
    description: "Read durable project issues, planning state, dependencies, and the current session link.",
    promptSnippet: "Query durable project work and dependency state",
    parameters: searchSchema,
    async execute(_id, input: SearchInput, signal, _update, ctx) {
      const request: Record<string, unknown> = { ...input };
      if (input.view === "session") request.sessionId = ctx.sessionManager.getSessionId();
      return { content: [{ type: "text", text: await run(pi, "search", request, ctx, signal) }] };
    },
  });

  pi.registerTool({
    name: "workgraph_edit",
    label: "Work graph edit",
    description: "Create or update durable project issues, notes, dependencies, status, and this Pi session's issue link.",
    promptSnippet: "Update durable project work and dependency state",
    promptGuidelines: ["Use workgraph_edit once work is concrete, record useful progress, and mark completed work done."],
    parameters: editSchema,
    async execute(toolCallId, input: EditInput, signal, _update, ctx) {
      const request: Record<string, unknown> = { ...input, idempotencyKey: `${ctx.sessionManager.getSessionId()}:${toolCallId}` };
      if (input.action === "link_session" || input.action === "unlink_session") {
        request.sessionId = ctx.sessionManager.getSessionId();
      }
      if (input.action === "link_session") {
        const path = ctx.sessionManager.getSessionFile();
        if (!path) throw new Error("the current Pi session is not durable yet");
        request.sessionPath = path;
      }
      return { content: [{ type: "text", text: await run(pi, "edit", request, ctx, signal) }] };
    },
  });
}
