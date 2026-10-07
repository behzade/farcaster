// One SDK instance owns each live agent and its runs. Rust consumes Connect/JSON.
import http from "node:http";
import { randomBytes } from "node:crypto";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const [sdkPath, ...args] = process.argv.slice(1);
const { Agent, Cursor } = await timed("SDK.import", () => import(pathToFileURL(sdkPath).href));
const { SqliteLocalAgentStore } = await timed("SqliteLocalAgentStore.import", () => import(new URL("./sqlite.js", pathToFileURL(sdkPath)).href));
const workspace = args[args.indexOf("--workspace") + 1];
const temporaryStore = args.includes("--temporary-store") ? args[args.indexOf("--temporary-store") + 1] : undefined;
const store = await timed("SqliteLocalAgentStore.open", () => SqliteLocalAgentStore.open({ workspaceRef: workspace, stateRoot: temporaryStore }));
timedSync("Cursor.configure", () => Cursor.configure({ local: { store } }));
const directory = await mkdtemp(join(tmpdir(), "farcaster-cursor-"));
const authTokenFile = join(directory, "token");
const token = randomBytes(32).toString("hex");
await writeFile(authTokenFile, token, { mode: 0o600 });
const agents = new Map();
const active = new Map();
let closing = false;
const stringify = (value) => JSON.stringify(value, (_, item) => typeof item === "bigint" ? item.toString() : item);
const localOptions = (wire = {}) => ({ ...wire, runtime: "local", cwd: wire.cwd ?? workspace, store });
const agentInfo = (info) => ({ ...info, local: { cwd: info.cwd ?? workspace },
  lastModified: new Date(info.lastModified).toISOString(),
  status: `AGENT_INFO_STATUS_${(info.status ?? "finished").toUpperCase()}` });

function agentOptions(wire) {
  const local = wire.local ?? {};
  const mcpServers = Object.fromEntries(Object.entries(wire.mcpServers ?? {}).map(([name, config]) => [name,
    config.http ? { ...config.http, type: "http" } : config.stdio ? { ...config.stdio, type: "stdio" } : config,
  ]));
  return {
    apiKey: wire.apiKey,
    model: wire.model,
    tools: temporaryStore ? [] : undefined,
    local: {
      cwd: local.cwd?.[0] ?? workspace,
      dirs: local.cwd?.slice(1),
      settingSources: temporaryStore ? [] : (local.settingSources ?? []).map((source) => source.replace("SETTING_SOURCE_", "").toLowerCase()),
      sandboxOptions: local.sandboxOptions,
      autoReview: local.autoReview,
      store,
    },
    mcpServers: temporaryStore ? {} : mcpServers,
  };
}
function frame(value, flags = 0) {
  const payload = Buffer.from(stringify(value));
  const header = Buffer.alloc(5);
  header[0] = flags;
  header.writeUInt32BE(payload.length, 1);
  return Buffer.concat([header, payload]);
}
function reply(response, value) {
  response.writeHead(200, { "Content-Type": "application/json", "Connection": "close" });
  response.end(stringify(value));
}
async function body(request) {
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > 32 * 1024 * 1024) throw new Error("Cursor request too large");
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}
async function send(wire, response) {
  const id = wire.agentId;
  if (active.has(id)) throw new Error("Cursor agent already has a live run");
  const entry = { run: null, disconnected: false };
  const runTiming = beginTiming("Run.lifecycle");
  let firstDelta = false;
  let firstMessage = false;
  let firstText = false;
  active.set(id, entry);
  response.writeHead(200, { "Content-Type": "application/connect+json", "Connection": "close" });
  response.on("close", () => {
    if (!response.writableFinished) {
      entry.disconnected = true;
      if (entry.run) void timed("Run.cancel", () => entry.run.cancel()).catch(() => {});
    }
  });
  // A quiet tool can run longer than the transport read timeout.
  const heartbeat = setInterval(() => {
    if (!response.destroyed && !response.writableNeedDrain) response.write(frame({}));
  }, 15000);
  try {
    const agent = agents.get(id);
    if (!agent) throw new Error("Cursor agent is not open");
    const emit = async (value) => {
      if (response.destroyed) throw new Error("Cursor client disconnected");
      if (!response.write(frame(value))) await new Promise((resolve, reject) => {
        const cleanup = () => { response.off("drain", drain); response.off("close", closed); };
        const drain = () => { cleanup(); resolve(); };
        const closed = () => { cleanup(); reject(new Error("Cursor client disconnected")); };
        response.once("drain", drain);
        response.once("close", closed);
      });
    };
    entry.run = await timed("Agent.send", () => agent.send({
      text: wire.message.text,
      images: (wire.message.images ?? []).map((image) => ({ data: image.data.data, mimeType: image.data.mimeType })),
    }, {
      model: wire.options?.model,
      mode: wire.options?.mode === "AGENT_MODE_OPTION_PLAN" ? "plan" : "agent",
      // One ordered event source for text, tools, and turn boundaries.
      onDelta: async ({ update }) => {
        if (!firstDelta) { firstDelta = true; runTiming("first_delta"); }
        if (!firstText && update.type === "text-delta") { firstText = true; runTiming("first_text"); }
        await emit({ interactionUpdate: update });
      },
    }));
    if (entry.disconnected) {
      await timed("Run.cancel", () => entry.run.cancel());
      throw new Error("Cursor client disconnected");
    }
    await emit({ runStarted: { runId: entry.run.id } });
    const stream = timedSync("Run.stream", () => entry.run.stream());
    const streamTiming = beginTiming("Run.stream.consume");
    try {
      for await (const message of stream) {
        if (!firstMessage) { firstMessage = true; runTiming("first_message"); }
        // These messages duplicate onDelta activity. Keep only SDK control/status.
        if (!["assistant", "thinking", "tool_call", "usage"].includes(message.type)) {
          await emit({ sdkMessage: { type: message.type, message } });
        }
      }
      streamTiming("ok");
    } catch (error) { streamTiming("error"); throw error; }
    const result = await timed("Run.wait", () => entry.run.wait());
    await emit({ result: {
      runId: entry.run.id,
      status: `RUN_LIFECYCLE_STATUS_${result.status.toUpperCase()}`,
      errorCode: result.error?.message,
      result,
    } });
    response.end(frame({}, 2));
    runTiming("ok");
  } catch (error) {
    runTiming("error");
    response.end(frame({ error: { code: "internal", message: error.message } }, 2));
  } finally {
    clearInterval(heartbeat);
    if (active.get(id) === entry) active.delete(id);
  }
}
const server = http.createServer(async (request, response) => {
  try {
    if (request.headers.authorization !== `Bearer ${token}`) {
      response.writeHead(401); response.end(); return;
    }
    const method = request.url.split("/").at(-1);
    const bytes = await body(request);
    const wire = JSON.parse((method === "Send" ? bytes.subarray(5) : bytes).toString());
    if (method === "Send") { await send(wire, response); return; }
    if (method === "SteerRun") {
      const run = active.get(wire.agentId)?.run;
      const outcome = run && (!wire.runId || wire.runId === run.id)
        ? await timed("Run.steer", () => run.steer?.(wire.text)) ?? "revert_to_followup"
        : "revert_to_followup";
      reply(response, { outcome }); return;
    }
    if (method === "CancelRun") {
      const run = active.get(wire.agentId)?.run;
      if (run && run.id === wire.runId) { await timed("Run.cancel", () => run.cancel()); reply(response, {}); return; }
    }
    let result;
    switch (method) {
      case "GetVersion": result = { protocolVersion: "sdk.v1" }; break;
      case "ListModels": result = { items: await timed("Cursor.models.list", () => Cursor.models.list(wire.options)) }; break;
      case "CreateAgent":
      case "ResumeAgent": {
        const agent = method === "CreateAgent"
          ? await timed("Agent.create", () => Agent.create(agentOptions(wire.options ?? {})))
          : await timed("Agent.resume", () => Agent.resume(wire.agentId, agentOptions(wire.options ?? {})));
        agents.set(agent.agentId, agent);
        result = { agentId: agent.agentId }; break;
      }
      case "GetAgent": result = { agent: agentInfo(await timed("Agent.get", () => Agent.get(wire.agentId, localOptions(wire.options)))) }; break;
      case "RenameAgent": {
        const agent = await timed("LocalAgentStore.agents.get", () => store.agents.get({ agentId: wire.agentId }));
        if (!agent) throw new Error("Cursor agent not found");
        await timed("LocalAgentStore.agents.update", () => store.agents.update({ agent: { ...agent, name: wire.name, updatedAt: Date.now() } }));
        result = {}; break;
      }
      case "ListRuns": {
        const page = await timed("Agent.listRuns", () => Agent.listRuns(wire.agentId, localOptions(wire.options)));
        result = { ...page, items: page.items.map(run => ({ runId: run.id, model: run.model })) }; break;
      }
      case "ListAgentMessages": result = { messages: await timed("Agent.messages.list", () => Agent.messages.list(wire.agentId, localOptions(wire.options))) }; break;
      case "CancelRun":
        if (active.has(wire.agentId)) throw new Error("Cursor run ID does not match the active run");
        result = {}; break;
      case "Shutdown": reply(response, {}); void close(); return;
      default: throw new Error(`Unknown Cursor operation: ${method}`);
    }
    reply(response, result);
  } catch (error) {
    if (response.headersSent) response.destroy();
    else { response.writeHead(500, { "Content-Type": "application/json" }); response.end(JSON.stringify({ code: "internal", message: error.message })); }
  }
});
async function close() {
  if (closing) return;
  closing = true;
  server.close();
  await Promise.allSettled([...active.values()].map(({ run }) => run && timed("Run.cancel", () => run.cancel())));
  for (const agent of agents.values()) await timed("Agent.close", () => agent.close());
  await rm(directory, { recursive: true, force: true });
  await timed("SqliteLocalAgentStore.dispose", () => store.dispose());
  process.exit(0);
}
process.on("SIGTERM", close);
process.on("SIGINT", close);
server.listen(0, "127.0.0.1", () => {
  process.stderr.write(`cursor-sdk-bridge ready ${JSON.stringify({ schemaVersion: 1, transport: "tcp", protocol: "connect", authTokenFile, url: `http://127.0.0.1:${server.address().port}` })}\n`);
});
