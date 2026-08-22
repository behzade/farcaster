import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { truncateHead, DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES } from "@earendil-works/pi-coding-agent";
import { StringEnum } from "@earendil-works/pi-ai";
import { Type, type Static } from "typebox";

const WORKGRAPH_RPC_TITLE = "\u001fpi-gpui-workgraph\u001f";

const completion = StringEnum(["revision_or_observation", "file", "observation"] as const);

const searchSchema = Type.Object({
  view: StringEnum(["project", "plan", "node", "session"] as const),
  plan: Type.Optional(Type.Integer({ minimum: 1 })),
  walk: Type.Optional(Type.Integer({ minimum: 1 })),
  number: Type.Optional(Type.Integer({ minimum: 1 })),
});

const editSchema = Type.Object({
  action: StringEnum([
    "create_plan",
    "add_node",
    "set_node",
    "add_edge",
    "remove_edge",
    "create_walk",
    "advance",
    "rewind",
    "link_session",
    "unlink_session",
  ] as const),
  title: Type.Optional(Type.String()),
  rootTitle: Type.Optional(Type.String()),
  files: Type.Optional(Type.Array(Type.String(), { maxItems: 64 })),
  completion: Type.Optional(completion),
  plan: Type.Optional(Type.Integer({ minimum: 1 })),
  walk: Type.Optional(Type.Integer({ minimum: 1 })),
  number: Type.Optional(Type.Integer({ minimum: 1 })),
  after: Type.Optional(Type.Integer({ minimum: 1 })),
  from: Type.Optional(Type.Integer({ minimum: 1 })),
  to: Type.Optional(Type.Integer({ minimum: 1 })),
  next: Type.Optional(Type.Integer({ minimum: 1 })),
  note: Type.Optional(Type.String({ maxLength: 1000 })),
  evidenceKind: Type.Optional(StringEnum(["revision", "file", "observation"] as const)),
  evidence: Type.Optional(Type.String({ maxLength: 4096 })),
  expectedVersion: Type.Optional(Type.Integer({ minimum: 1 })),
});

type SearchInput = Static<typeof searchSchema>;
type EditInput = Static<typeof editSchema>;

function fields(input: Record<string, unknown>): Record<string, string> {
  return Object.fromEntries(
    Object.entries(input)
      .filter(([, value]) => value !== undefined)
      .map(([key, value]) => [key, Array.isArray(value) ? JSON.stringify(value) : String(value)]),
  );
}

async function run(
  operation: "search" | "edit",
  input: Record<string, unknown>,
  ctx: ExtensionContext,
): Promise<string> {
  const value = await ctx.ui.input(
    WORKGRAPH_RPC_TITLE,
    JSON.stringify({ operation, project: ctx.cwd, fields: fields(input) }),
  );
  if (value === undefined) throw new Error("Pi GPUI cancelled the work graph request");
  const result = JSON.parse(value) as { success?: boolean; error?: string };
  if (!result.success) throw new Error(result.error || `work graph ${operation} failed`);
  const truncated = truncateHead(value, { maxBytes: DEFAULT_MAX_BYTES, maxLines: DEFAULT_MAX_LINES });
  return truncated.truncated ? `${truncated.content}\n[Work graph output truncated]` : truncated.content;
}

export default function workgraph(pi: ExtensionAPI): void {
  pi.registerTool({
    name: "workgraph_search",
    label: "Work graph search",
    description: "Read durable project plans, state nodes, walks, outcomes, and the current session attachment.",
    promptSnippet: "Read durable project plans and the current walk position",
    parameters: searchSchema,
    async execute(_id, input: SearchInput, _signal, _update, ctx) {
      const request: Record<string, unknown> = { ...input };
      if (input.view === "session") request.sessionId = ctx.sessionManager.getSessionId();
      return { content: [{ type: "text", text: await run("search", request, ctx) }] };
    },
  });

  pi.registerTool({
    name: "workgraph_edit",
    label: "Work graph edit",
    description: "Create or change durable plan nodes and edges, advance or rewind a walk with one bounded outcome, and attach this session.",
    promptSnippet: "Change durable plans and record walk outcomes",
    promptGuidelines: [
      "Use workgraph_edit when concrete work needs a durable plan; keep node titles, scoped paths, and outcome notes concise.",
      "Use workgraph_edit action advance only after its evidence satisfies the current node; use observation evidence when the state already exists.",
      "Use workgraph_edit action link_session to attach the current Pi session to the walk being continued.",
    ],
    parameters: editSchema,
    async execute(toolCallId, input: EditInput, _signal, _update, ctx) {
      const request: Record<string, unknown> = {
        ...input,
        idempotencyKey: `${ctx.sessionManager.getSessionId()}:${toolCallId}`,
      };
      if (input.action === "link_session" || input.action === "unlink_session") {
        request.sessionId = ctx.sessionManager.getSessionId();
      }
      if (input.action === "link_session") {
        const path = ctx.sessionManager.getSessionFile();
        if (!path) throw new Error("the current Pi session is not durable yet");
        request.sessionPath = path;
      }
      return { content: [{ type: "text", text: await run("edit", request, ctx) }] };
    },
  });
}
