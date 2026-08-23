import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { truncateHead, DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES } from "@earendil-works/pi-coding-agent";
import { StringEnum } from "@earendil-works/pi-ai";
import { Type, type Static } from "typebox";

const WORKGRAPH_RPC_TITLE = "\u001fpi-gpui-workgraph\u001f";

const node = Type.Object(
  {
    title: Type.String({ minLength: 1, maxLength: 512 }),
    acceptance: Type.String({ minLength: 1, maxLength: 4096 }),
  },
  { additionalProperties: false },
);

const parameters = Type.Object(
  {
    action: StringEnum(["search", "patch", "complete"] as const),
    query: Type.Optional(
      Type.String({ description: "Search titles and acceptance conditions", maxLength: 512 }),
    ),
    nodes: Type.Optional(
      Type.Array(node, {
        description: "Ordered chain required for patch",
        minItems: 1,
        maxItems: 64,
      }),
    ),
    after: Type.Optional(
      Type.Integer({ description: "Attach the first new node after this node", minimum: 1 }),
    ),
    before: Type.Optional(
      Type.Integer({ description: "Attach the last new node before this node", minimum: 1 }),
    ),
    evidence: Type.Optional(
      Type.String({
        description: "Acceptance evidence required for complete",
        minLength: 1,
        maxLength: 4096,
      }),
    ),
    next: Type.Optional(
      Type.Integer({
        description: "Successor to activate when the current node branches",
        minimum: 1,
      }),
    ),
  },
  { additionalProperties: false },
);

type WorkgraphInput = Static<typeof parameters>;

function fields(input: Record<string, unknown>): Record<string, string> {
  return Object.fromEntries(
    Object.entries(input)
      .filter(([, value]) => value !== undefined)
      .map(([key, value]) => [key, Array.isArray(value) ? JSON.stringify(value) : String(value)]),
  );
}

async function run(input: Record<string, unknown>, ctx: ExtensionContext): Promise<string> {
  const value = await ctx.ui.input(
    WORKGRAPH_RPC_TITLE,
    JSON.stringify({ operation: "workgraph", project: ctx.cwd, fields: fields(input) }),
  );
  if (value === undefined) throw new Error("Pi GPUI cancelled the work graph request");
  const result = JSON.parse(value) as { success?: boolean; error?: string };
  if (!result.success) throw new Error(result.error || "work graph request failed");
  const truncated = truncateHead(value, { maxBytes: DEFAULT_MAX_BYTES, maxLines: DEFAULT_MAX_LINES });
  return truncated.truncated ? `${truncated.content}\n[Work graph output truncated]` : truncated.content;
}

export default function workgraph(pi: ExtensionAPI): void {
  pi.registerTool({
    name: "workgraph",
    label: "Work graph",
    description:
      "Search or change the current session's durable work graph. Patch adds an ordered node chain: " +
      "after attaches the first new node, before attaches the last, and omitting both creates a graph. " +
      "Complete records evidence for the active node and advances it.",
    promptSnippet: "Search, patch, or complete nodes in the current session's durable work graph",
    promptGuidelines: [
      "Use workgraph for concrete multi-stage work; each node needs a concise title and observable acceptance condition.",
      "Use workgraph action complete only after its evidence satisfies the active node's acceptance condition.",
    ],
    parameters,
    async execute(toolCallId, input: WorkgraphInput, _signal, _update, ctx) {
      const sessionId = ctx.sessionManager.getSessionId();
      const request: Record<string, unknown> = { ...input, sessionId };
      if (input.action !== "search") {
        request.idempotencyKey = `${sessionId}:${toolCallId}`;
      }
      if (input.action === "patch") {
        const path = ctx.sessionManager.getSessionFile();
        if (!path) throw new Error("the current Pi session is not durable yet");
        request.sessionPath = path;
      }
      return { content: [{ type: "text", text: await run(request, ctx) }] };
    },
  });
}
