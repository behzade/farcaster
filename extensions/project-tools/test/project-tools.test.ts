import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { Cause, Effect, Exit } from "effect";
import projectTools from "../src/index.ts";
import { discoverProjectTools } from "../src/discovery.ts";
import { executeProjectTool, formatProjectToolResult } from "../src/module.ts";

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

async function makeProject(options: ToolOptions = {}) {
  const root = await mkdtemp(join(tmpdir(), "pi-project-tools-"));
  const directory = join(root, ".pi", "tools", "example");
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
  assert.equal(formatProjectToolResult(value), '{\n  "value": "ok:host-access"\n}');
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
  assert.equal(formatProjectToolResult(value), "plain");

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

  await sessionStart!({ type: "session_start", reason: "new" }, {
    ...context,
    isProjectTrusted: () => true,
  });
  assert.equal(tools.length, 1);
});
