import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import test from "node:test";

const patchPath = new URL("../patches/pi-subagents-local-hardening.patch", import.meta.url);
const packagePath = new URL("../nix/pi-subagents.nix", import.meta.url);
const installedPackage = process.env.PI_SUBAGENTS_PACKAGE;

test("packaged subagents bound every final model-visible output path", async () => {
  const [patch, packageExpression] = await Promise.all([
    readFile(patchPath, "utf8"),
    readFile(packagePath, "utf8"),
  ]);

  assert.match(packageExpression, /pi-subagents-local-hardening\.patch/);
  for (const source of [
    "src/shared/types.ts",
    "src/runs/foreground/execution.ts",
    "src/runs/foreground/subagent-executor.ts",
    "src/runs/background/subagent-runner.ts",
    "src/runs/background/notify.ts",
    "src/intercom/result-intercom.ts",
  ]) {
    assert.ok(patch.includes(source), `missing output-bound patch for ${source}`);
  }
  assert.match(patch, /boundForegroundModelOutput\(/);
  assert.match(patch, /const finalizeForegroundResult = .*boundForegroundModelOutput/s);
  assert.match(patch, /const errorResult = boundForegroundModelOutput\(/);
  assert.match(patch, /candidate && fs\.existsSync\(candidate\)/);
  assert.match(patch, /options\.artifactConfig\?\.includeOutput !== false/);
  assert.match(patch, /output: truncateOutput\(r\.output, outputConfig/);
  assert.match(patch, /payload\.message = truncateOutput/);
  assert.match(patch, /const content = truncateOutput\(formatted, DEFAULT_MAX_OUTPUT\)\.text/);
  assert.doesNotMatch(patch, /^\+\s*if \((?:options\.)?maxOutput\)/m);
});

test("installed subagent truncation includes its marker inside hard byte and line caps", {
  skip: installedPackage ? false : "PI_SUBAGENTS_PACKAGE is set by the Nix integration check",
}, async () => {
  const shared = await import(pathToFileURL(join(installedPackage!, "src/shared/types.ts")).href) as {
    truncateOutput(output: string, config: { bytes: number; lines: number }, artifactPath?: string): { text: string; truncated: boolean };
  };
  const cases = [
    { output: "x".repeat(300_000), config: { bytes: 200 * 1024, lines: 5000 } },
    { output: Array.from({ length: 6000 }, (_, index) => `line-${index}`).join("\n"), config: { bytes: 200 * 1024, lines: 5000 } },
    { output: "界".repeat(100_000), config: { bytes: 200 * 1024, lines: 5000 } },
    { output: "abc", config: { bytes: 1, lines: 1 } },
  ];

  for (const { output, config } of cases) {
    const result = shared.truncateOutput(output, config);
    assert.equal(result.truncated, true);
    assert.ok(Buffer.byteLength(result.text) <= config.bytes);
    assert.ok(result.text.split("\n").length <= config.lines);
    assert.doesNotMatch(result.text, /�/);
  }
});

test("installed grouped intercom messages have one global output cap", {
  skip: installedPackage ? false : "PI_SUBAGENTS_PACKAGE is set by the Nix integration check",
}, async () => {
  const intercom = await import(pathToFileURL(join(installedPackage!, "src/intercom/result-intercom.ts")).href) as {
    buildSubagentResultIntercomPayload(input: Record<string, unknown>): { message: string };
  };
  const payload = intercom.buildSubagentResultIntercomPayload({
    to: "parent",
    runId: "run-1",
    mode: "parallel",
    source: "async",
    children: Array.from({ length: 8 }, (_, index) => ({
      agent: `child-${index}`,
      status: "completed",
      summary: "界".repeat(100_000),
    })),
  });

  assert.ok(Buffer.byteLength(payload.message) <= 200 * 1024);
  assert.ok(payload.message.split("\n").length <= 5000);
  assert.doesNotMatch(payload.message, /�/);
  assert.match(payload.message, /^\[TRUNCATED:/);
});

test("installed async completion notifications have one global output cap", {
  skip: installedPackage ? false : "PI_SUBAGENTS_PACKAGE is set by the Nix integration check",
}, async () => {
  const module = await import(pathToFileURL(join(installedPackage!, "src/runs/background/notify.ts")).href) as {
    default: (pi: Record<string, unknown>, state: { currentSessionId: string }) => {
      deliver(result: Record<string, unknown>): Promise<boolean>;
      dispose(): void;
    };
  };
  let content = "";
  const notifier = module.default({
    sendMessage(message: { content: string }) {
      content = message.content;
    },
    events: { on: () => () => {} },
  }, { currentSessionId: "session-1" });

  const accepted = await notifier.deliver({
    id: "run-1",
    sessionId: "session-1",
    agent: "worker",
    success: false,
    summary: "界".repeat(100_000),
  });
  notifier.dispose();

  assert.equal(accepted, true);
  assert.ok(Buffer.byteLength(content) <= 200 * 1024);
  assert.ok(content.split("\n").length <= 5000);
  assert.doesNotMatch(content, /�/);
  assert.match(content, /^\[TRUNCATED:/);
});
