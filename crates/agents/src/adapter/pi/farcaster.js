const marker = "farcaster-steering-resume";
const protocolVersion = "2026-07-28";

export default async function steering(pi) {
  registerSteering(pi);
  const boundary = process.env.FARCASTER_PROMPT_BOUNDARY_URL;
  if (boundary) {
    // Pi awaits turn_end after all tools, before reading steering messages.
    // The host releases this request after native steer admission, not delivery.
    pi.on("turn_end", async (_event, ctx) => {
      const response = await fetch(boundary, {
        method: "POST",
        headers: {"Content-Type": "application/json"},
        body: JSON.stringify({session_id: ctx.sessionManager.getSessionId()}),
      });
      if (!response.ok) throw new Error("Farcaster prompt boundary unavailable");
      await response.json();
    });
  }
  const url = process.env.FARCASTER_MCP_URL;
  const token = process.env.FARCASTER_MCP_CALLER;
  const header = process.env.FARCASTER_MCP_HEADER || "farcaster-caller";
  let failure;
  if (!url || !token) {
    if (process.env.FARCASTER_PROCESS_ROLE === "session") {
      failure = `missing FARCASTER_MCP env (url=${url ? "set" : "unset"}, caller=${token ? "set" : "unset"})`;
    }
  } else {
    try {
      await registerFarcasterTools(pi, url, token, header);
    } catch (error) {
      failure = error instanceof Error ? error.message : String(error);
    }
  }
  if (failure) {
    console.error(`Farcaster tools: ${failure}`);
    pi.on("session_start", () => {
      pi.appendEntry("farcaster-tools-error", {error: failure});
    });
  }
}

function registerSteering(pi) {
  let applying = false;
  let resumeStarted;
  pi.on("agent_start", () => {
    resumeStarted?.();
    resumeStarted = undefined;
  });
  pi.on("context", (event) => ({
    messages: event.messages.filter(message => message.customType !== marker),
  }));
  pi.registerCommand("farcaster-apply-steering", {
    description: "Apply pending Farcaster input",
    handler: async (_args, ctx) => {
      if (applying || (ctx.isIdle() && !ctx.hasPendingMessages())) return;
      applying = true;
      try {
        ctx.abort();
        await ctx.waitForIdle();
        // Start a new run without duplicating input already consumed before abort.
        // The context hook removes this hidden trigger before the provider sees it.
        await new Promise(resolve => {
          resumeStarted = resolve;
          pi.sendMessage({customType: marker, content: [], display: false}, {triggerTurn: true});
        });
      } finally {
        applying = false;
      }
    },
  });
}

async function registerFarcasterTools(pi, url, token, header) {
  const client = new McpClient(url, token, header);
  const initialize = await client.request("initialize", {
    protocolVersion,
    capabilities: {},
    clientInfo: {name: "farcaster-pi", version: "0.1.0"},
  });
  await client.notify("notifications/initialized");
  const listed = await client.request("tools/list", {});
  const tools = Array.isArray(listed?.tools) ? listed.tools : [];
  for (const tool of tools) {
    if (!tool?.name) continue;
    pi.registerTool({
      name: `farcaster_${tool.name}`,
      label: toolLabel(tool.name),
      description: tool.description || tool.name,
      parameters: tool.inputSchema || {type: "object", properties: {}},
      async execute(_toolCallId, params, signal) {
        return toPiResult(await client.request("tools/call", {
          name: tool.name,
          arguments: params ?? {},
        }, signal));
      },
    });
  }
  const instructions = initialize?.instructions;
  if (typeof instructions === "string" && instructions.trim()) {
    pi.on("before_agent_start", (event) => {
      if (event.systemPrompt.includes(instructions)) return;
      return {systemPrompt: `${event.systemPrompt}\n\n${instructions}`};
    });
  }
}

// SEP-2243 (2026-07-28): every non-initialize POST declares its JSON-RPC method
// and, for methods with a routing target, that target, so middle boxes can
// route without parsing bodies.
function standardHeaders(body) {
  const method = body?.method;
  if (typeof method !== "string" || !method) return {};
  const headers = {"Mcp-Method": method};
  if (method === "tools/call" && typeof body.params?.name === "string") {
    headers["Mcp-Name"] = encodeHeaderValue(body.params.name);
  }
  return headers;
}

function withRequestMeta(params) {
  return {
    _meta: {
      "io.modelcontextprotocol/protocolVersion": protocolVersion,
      "io.modelcontextprotocol/clientCapabilities": {},
    },
    ...params,
  };
}

function encodeHeaderValue(value) {
  const needsBase64 = /^[ \t]/.test(value)
    || /[ \t]$/.test(value)
    || /[^\x20-\x7E]/.test(value)
    || (value.startsWith("=?base64?") && value.endsWith("?="));
  if (!needsBase64) return value;
  return `=?base64?${Buffer.from(value, "utf8").toString("base64")}?=`;
}

function toolLabel(name) {
  return name
    .split("_")
    .filter(Boolean)
    .map(word => word[0].toUpperCase() + word.slice(1))
    .join(" ");
}

// The old MCP gateway truncated oversized results; pass-through would otherwise
// push unbounded JSON (full workgraph dumps) into model context.
const MAX_TEXT_CHARS = 24_000;

function toPiResult(mcp) {
  const structured = mcp?.structuredContent;
  let content = Array.isArray(mcp?.content) ? mcp.content : [];
  const hasArtifactText = content.some(block =>
    block?.type === "text" && typeof block.text === "string" && block.text.includes("farcaster_review"),
  );
  if (structured && !hasArtifactText) {
    content = content.concat([{type: "text", text: JSON.stringify(structured)}]);
  }
  if (content.length === 0) {
    content = [{type: "text", text: JSON.stringify(mcp ?? {})}];
  }
  for (const block of content) {
    if (block?.type === "text" && block.text.length > MAX_TEXT_CHARS) {
      block.text = `${block.text.slice(0, MAX_TEXT_CHARS)}\n\n[Truncated, ${block.text.length} chars total]`;
    }
  }
  return {
    content,
    details: structured ?? {},
    isError: Boolean(mcp?.isError),
  };
}

class McpClient {
  constructor(url, token, header) {
    this.url = url;
    this.token = token;
    this.header = header;
    this.session = undefined;
    this.nextId = 1;
  }

  notify(method, params) {
    return this.send({jsonrpc: "2.0", method, params}, undefined);
  }

  async request(method, params, signal) {
    const payload = await this.send({
      jsonrpc: "2.0",
      id: this.nextId++,
      method,
      params: method === "initialize" ? params : withRequestMeta(params),
    }, signal);
    if (payload?.error) {
      throw new Error(payload.error.message || `${method} failed`);
    }
    return payload?.result;
  }

  async send(body, signal) {
    const headers = {
      "Accept": "application/json, text/event-stream",
      "Content-Type": "application/json",
      "MCP-Protocol-Version": protocolVersion,
      [this.header]: this.token,
    };
    if (this.session) headers["Mcp-Session-Id"] = this.session;
    Object.assign(headers, standardHeaders(body));
    const timeout = AbortSignal.timeout(8000);
    const response = await fetch(this.url, {
      method: "POST",
      headers,
      body: JSON.stringify(body),
      signal: signal ? AbortSignal.any([signal, timeout]) : timeout,
    });
    const session = response.headers.get("mcp-session-id");
    if (session) this.session = session;
    const text = await response.text();
    if (!response.ok) {
      throw new Error(`MCP HTTP ${response.status}: ${text.slice(0, 200)}`);
    }
    if (!text.trim()) return {};
    const type = response.headers.get("content-type") || "";
    if (type.includes("event-stream")) {
      const line = text.split("\n").find(row => row.startsWith("data: "));
      if (!line) throw new Error("MCP stream was empty");
      return JSON.parse(line.slice(6));
    }
    return JSON.parse(text);
  }
}
