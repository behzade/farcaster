import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { Cause, Effect, Exit } from "effect";
import projectTools from "../src/index.ts";
import { discoverProjectTools } from "../src/discovery.ts";
import { executeProjectTool, formatProjectToolResult } from "../src/module.ts";
import { PROJECT_TOOL_MAX_BYTES, PROJECT_TOOL_MAX_LINES } from "../src/truncation.ts";

interface ToolOptions {
  readonly result?: unknown;
  readonly extraManifest?: Record<string, unknown>;
  readonly main?: string;
}

const objectResult = {
  type: "object",
  additionalProperties: false,
  required: ["value"],
  properties: { value: { type: "string" } },
};

async function makeProject(options: ToolOptions = {}, controlDirectory = "project-tools") {
  const root = await mkdtemp(join(tmpdir(), "pi-project-tools-"));
  const directory = join(root, ".pi", controlDirectory, "example");
  await mkdir(directory, { recursive: true });
  const manifest = {
    version: 1,
    name: "example",
    label: "Example",
    description: "Test a project tool",
    entrypoint: "main.ts",
    parameters: {
      type: "object",
      additionalProperties: false,
      required: ["input"],
      properties: { input: { type: "string" } },
    },
    result: options.result ?? objectResult,
    ...options.extraManifest,
  };
  await writeFile(join(directory, "tool.json"), JSON.stringify(manifest));
  await writeFile(join(directory, "main.ts"), options.main ?? [
    'import { Effect } from "effect";',
    "export const execute = (args: { input: string }) => Effect.succeed({ value: args.input });",
  ].join("\n"));
  return { root, cleanup: () => rm(root, { recursive: true, force: true }) };
}

test("ignores Pi's deprecated .pi/tools directory", async (t) => {
  const project = await makeProject({}, "tools");
  t.after(project.cleanup);

  const discovery = await Effect.runPromise(discoverProjectTools(project.root));
  assert.deepEqual(discovery.tools, []);
  assert.deepEqual(discovery.diagnostics, []);
});

async function discoverOne(root: string) {
  const discovery = await Effect.runPromise(discoverProjectTools(root));
  assert.deepEqual(discovery.diagnostics, []);
  assert.equal(discovery.tools.length, 1);
  return discovery.tools[0]!;
}

test("loads and executes an Effect v4 project tool in the host", async (t) => {
  const project = await makeProject({
    main: [
      'import { Effect } from "effect";',
      'import { readFile } from "node:fs/promises";',
      "export const execute = (args: { input: string }, context: { projectRoot: string }) =>",
      "  Effect.promise(async () => ({ value: `${args.input}:${await readFile(`${context.projectRoot}/host.txt`, \"utf8\")}` }));",
    ].join("\n"),
  });
  t.after(project.cleanup);
  await writeFile(join(project.root, "host.txt"), "host-access");

  const tool = await discoverOne(project.root);
  const value = await Effect.runPromise(executeProjectTool(tool, { input: "ok" }, {
    toolCallId: "call-1",
    projectRoot: project.root,
    signal: undefined,
  }));
  assert.deepEqual(value, { value: "ok:host-access" });
  assert.deepEqual(formatProjectToolResult(value), { text: '{\n  "value": "ok:host-access"\n}' });
});

function outputLineCount(value: string): number {
  if (value.length === 0) return 0;
  return value.endsWith("\n") ? value.split("\n").length - 1 : value.split("\n").length;
}

test("truncates oversized project tool results instead of failing", () => {
  const byteLimited = formatProjectToolResult(`start:${"🙂".repeat(PROJECT_TOOL_MAX_BYTES)}:end`);
  assert.ok(byteLimited.truncation);
  assert.match(byteLimited.text, /^start:/);
  assert.doesNotMatch(byteLimited.text, /:end/);
  assert.match(byteLimited.text, /\[Project tool output truncated:/);
  assert.ok(Buffer.byteLength(byteLimited.text, "utf8") <= PROJECT_TOOL_MAX_BYTES);

  const lineLimited = formatProjectToolResult(
    Array.from({ length: PROJECT_TOOL_MAX_LINES + 100 }, (_, index) => `line-${index}`).join("\n"),
  );
  assert.ok(lineLimited.truncation);
  assert.match(lineLimited.text, /^line-0\n/);
  assert.doesNotMatch(lineLimited.text, /line-2099/);
  assert.match(lineLimited.text, /\[Project tool output truncated:/);
  assert.ok(outputLineCount(lineLimited.text) <= PROJECT_TOOL_MAX_LINES);
});

test("provides an exported Effect layer to declared dependencies", async (t) => {
  const project = await makeProject({
    main: [
      'import { Context, Effect, Layer } from "effect";',
      'const Prefix = Context.Service<{ value: string }>("Test/Prefix");',
      'export const dependencies = Layer.succeed(Prefix)({ value: "layer" });',
      "export const execute = (args: { input: string }) => Effect.gen(function* () {",
      "  const prefix = yield* Prefix;",
      "  return { value: `${prefix.value}:${args.input}` };",
      "});",
    ].join("\n"),
  });
  t.after(project.cleanup);
  const tool = await discoverOne(project.root);
  const value = await Effect.runPromise(executeProjectTool(tool, { input: "ok" }, {
    toolCallId: "call-2",
    projectRoot: project.root,
    signal: undefined,
  }));
  assert.deepEqual(value, { value: "layer:ok" });
});

test("loads Effect v4 subpath imports from project tools", async (t) => {
  const project = await makeProject({
    main: [
      'import { Effect } from "effect";',
      'import { FetchHttpClient } from "effect/unstable/http";',
      "export const dependencies = FetchHttpClient.layer;",
      "export const execute = (args: { input: string }) => Effect.succeed({ value: args.input });",
    ].join("\n"),
  });
  t.after(project.cleanup);

  const tool = await discoverOne(project.root);
  const value = await Effect.runPromise(executeProjectTool(tool, { input: "subpath" }, {
    toolCallId: "call-subpath",
    projectRoot: project.root,
    signal: undefined,
  }));
  assert.deepEqual(value, { value: "subpath" });
});

test("uses the Effect failure exit as the tool error", async (t) => {
  const project = await makeProject({
    main: [
      'import { Effect } from "effect";',
      'export const execute = () => Effect.fail({ _tag: "ExpectedError", reason: "denied" });',
    ].join("\n"),
  });
  t.after(project.cleanup);
  const tool = await discoverOne(project.root);
  const exit = await Effect.runPromiseExit(executeProjectTool(tool, { input: "ok" }, {
    toolCallId: "call-3",
    projectRoot: project.root,
    signal: undefined,
  }));
  assert.ok(Exit.isFailure(exit));
  assert.match(String(Cause.squash(exit.cause)), /denied/);
});

test("cancels the Effect when Pi aborts the tool call", async (t) => {
  const project = await makeProject({
    main: 'import { Effect } from "effect"; export const execute = () => Effect.never;',
  });
  t.after(project.cleanup);
  const tool = await discoverOne(project.root);
  const controller = new AbortController();
  const running = Effect.runPromise(executeProjectTool(tool, { input: "ok" }, {
    toolCallId: "call-cancel",
    projectRoot: project.root,
    signal: controller.signal,
  }));
  controller.abort();
  await assert.rejects(running, /execution cancelled/);
});

test("aborting a project tool releases its dependency Layer", async (t) => {
  const project = await makeProject();
  t.after(project.cleanup);
  const releasedPath = join(project.root, "released.txt");
  await writeFile(join(project.root, ".pi", "project-tools", "example", "main.ts"), [
    'import { Context, Effect, Layer } from "effect";',
    'import { writeFile } from "node:fs/promises";',
    'const Resource = Context.Service<{ ready: true }>("Test/Resource");',
    "export const dependencies = Layer.effect(Resource,",
    "  Effect.acquireRelease(",
    "    Effect.succeed({ ready: true as const }),",
    `    () => Effect.promise(() => writeFile(${JSON.stringify(releasedPath)}, "released")),`,
    "  ),",
    ");",
    "export const execute = () => Effect.never;",
  ].join("\n"));

  const tool = await discoverOne(project.root);
  const controller = new AbortController();
  const running = Effect.runPromise(executeProjectTool(tool, { input: "ok" }, {
    toolCallId: "call-finalizer",
    projectRoot: project.root,
    signal: controller.signal,
  }));
  controller.abort();
  await assert.rejects(running, /execution cancelled/);
  assert.equal(await readFile(releasedPath, "utf8"), "released");
});

test("accepts a plain string only when the result schema accepts a string", async (t) => {
  const accepted = await makeProject({
    result: { type: "string" },
    main: 'import { Effect } from "effect"; export const execute = () => Effect.succeed("plain");',
  });
  t.after(accepted.cleanup);
  const acceptedTool = await discoverOne(accepted.root);
  const value = await Effect.runPromise(executeProjectTool(acceptedTool, { input: "ok" }, {
    toolCallId: "call-4",
    projectRoot: accepted.root,
    signal: undefined,
  }));
  assert.equal(value, "plain");
  assert.deepEqual(formatProjectToolResult(value), { text: "plain" });

  const rejected = await makeProject({
    main: 'import { Effect } from "effect"; export const execute = () => Effect.succeed("plain");',
  });
  t.after(rejected.cleanup);
  const rejectedTool = await discoverOne(rejected.root);
  await assert.rejects(
    Effect.runPromise(executeProjectTool(rejectedTool, { input: "ok" }, {
      toolCallId: "call-5",
      projectRoot: rejected.root,
      signal: undefined,
    })),
    /invalid result/,
  );
});

test("keeps invalid manifests and loose nested object schemas inactive", async (t) => {
  const unknownField = await makeProject({ extraManifest: { permissions: {} } });
  t.after(unknownField.cleanup);
  const unknownDiscovery = await Effect.runPromise(discoverProjectTools(unknownField.root));
  assert.equal(unknownDiscovery.tools.length, 0);
  assert.match(unknownDiscovery.diagnostics[0]!.message, /unknown manifest field/);

  const looseObject = await makeProject({
    result: {
      type: "array",
      items: { type: "object", properties: {}, required: [] },
    },
  });
  t.after(looseObject.cleanup);
  const looseDiscovery = await Effect.runPromise(discoverProjectTools(looseObject.root));
  assert.equal(looseDiscovery.tools.length, 0);
  assert.match(looseDiscovery.diagnostics[0]!.message, /additionalProperties/);
});

test("requires execute to return an Effect", async (t) => {
  const project = await makeProject({ main: "export const execute = () => ({ value: 'not-effect' });" });
  t.after(project.cleanup);
  const tool = await discoverOne(project.root);
  await assert.rejects(
    Effect.runPromise(executeProjectTool(tool, { input: "ok" }, {
      toolCallId: "call-6",
      projectRoot: project.root,
      signal: undefined,
    })),
    /must return an Effect/,
  );
});

test("registers tools only for a trusted project and makes them active", async (t) => {
  const project = await makeProject();
  t.after(project.cleanup);
  let sessionStart: ((event: unknown, context: any) => Promise<void>) | undefined;
  const tools: any[] = [];
  let active: string[] = [];
  const pi = {
    on(event: string, handler: typeof sessionStart) {
      if (event === "session_start") sessionStart = handler;
    },
    registerTool(tool: unknown) {
      tools.push(tool);
    },
    getAllTools: () => tools,
    getActiveTools: () => active,
    setActiveTools(names: string[]) {
      active = names;
    },
  };
  projectTools(pi as never);
  assert.ok(sessionStart);
  const context = {
    cwd: project.root,
    hasUI: false,
    isProjectTrusted: () => false,
    ui: { notify() {} },
  };
  await sessionStart!({ type: "session_start", reason: "startup" }, context);
  assert.equal(tools.length, 0);

  await sessionStart!({ type: "session_start", reason: "reload" }, {
    ...context,
    isProjectTrusted: () => true,
  });
  assert.equal(tools.length, 1);
  assert.equal(active.length, 1);
  assert.match(active[0]!, /^project_pi_project_tools_[a-z0-9]+_example$/);
  assert.match(tools[0].description, /truncated to 2000 lines or 50KB/);

  const result = await tools[0].execute("bounded-call", { input: "x".repeat(PROJECT_TOOL_MAX_BYTES * 2) });
  assert.ok(Buffer.byteLength(result.content[0].text, "utf8") <= PROJECT_TOOL_MAX_BYTES);
  assert.match(result.content[0].text, /\[Project tool output truncated:/);
  assert.equal(result.details.projectTool, "example");
  assert.equal(result.details.truncation.maxBytes, PROJECT_TOOL_MAX_BYTES);

  await sessionStart!({ type: "session_start", reason: "new" }, {
    ...context,
    isProjectTrusted: () => true,
  });
  assert.equal(tools.length, 1);
});
