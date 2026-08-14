import assert from "node:assert/strict";
import test from "node:test";
import { buildPromptReport, formatPromptReport } from "../extensions/prompt-inspector.ts";

const input = {
  systemPrompt: "system prompt",
  activeToolNames: ["second", "first", "missing"],
  tools: [
    { name: "first", description: "short", parameters: { type: "object" } },
    {
      name: "second",
      description: "a larger definition",
      parameters: { type: "object", properties: { value: { type: "string" } } },
    },
    { name: "inactive", description: "not active", parameters: {} },
  ],
} as const;

test("prompt report preserves runtime tool order and measures only active definitions", () => {
  const report = buildPromptReport(input);

  assert.deepEqual(report.activeToolNames, ["second", "first", "missing"]);
  assert.deepEqual(report.activeDefinitions.map((tool) => tool.name), ["second", "first"]);
  assert.doesNotMatch(report.serializedActiveDefinitions, /inactive/);
  assert.equal(report.activeDefinitionCharacters, report.serializedActiveDefinitions.length);
  assert.equal(report.systemPromptCharacters, input.systemPrompt.length);
  assert.equal(report.roughSystemPromptTokens, Math.ceil(input.systemPrompt.length / 4));
  assert.equal(report.largestSchemaContributors[0]?.name, "second");
});

test("prompt report and fingerprints are deterministic", () => {
  const first = buildPromptReport(input);
  const second = buildPromptReport({
    systemPrompt: input.systemPrompt,
    activeToolNames: [...input.activeToolNames],
    tools: input.tools.map((tool) => ({ ...tool })),
  });

  assert.deepEqual(second, first);
  assert.match(first.systemPromptSha256, /^[a-f0-9]{64}$/);
  assert.match(first.activeDefinitionsSha256, /^[a-f0-9]{64}$/);
});

test("formatted report labels estimates and full mode exposes exact inputs", () => {
  const report = buildPromptReport(input);
  const summary = formatPromptReport(report, false);
  const full = formatPromptReport(report, true);

  assert.match(summary, /rough character heuristics, not provider token counts or billing data/);
  assert.doesNotMatch(summary, /# Exact effective system prompt/);
  assert.match(full, /# Exact effective system prompt\n\nsystem prompt/);
  assert.match(full, new RegExp(report.serializedActiveDefinitions.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
});
