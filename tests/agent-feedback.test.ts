import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, statSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  appendAgentFeedback,
  type AgentFeedbackRecord,
} from "../extensions/lib/agent-feedback.ts";

function record(id: string): AgentFeedbackRecord {
  return {
    version: 1,
    id,
    timestamp: "2026-08-15T00:00:00.000Z",
    category: "setup",
    severity: "blocking",
    summary: `feedback ${id}`,
    details: "The agent hit a concrete Pi setup failure.",
    cwd: "/workspace",
    agent: "scout",
    toolCallId: `call-${id}`,
  };
}

test("feedback records append as private JSONL entries", () => {
  const root = mkdtempSync(join(tmpdir(), "pi-agent-feedback-"));
  const path = join(root, "state", "agent-feedback.jsonl");
  try {
    appendAgentFeedback(path, record("first"));
    appendAgentFeedback(path, record("second"));

    const entries = readFileSync(path, "utf8").trimEnd().split("\n").map((line) => JSON.parse(line));
    assert.deepEqual(entries, [record("first"), record("second")]);
    assert.equal(statSync(path).mode & 0o777, 0o600);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("feedback append refuses a symlink destination", () => {
  const root = mkdtempSync(join(tmpdir(), "pi-agent-feedback-link-"));
  const target = join(root, "target.jsonl");
  const path = join(root, "agent-feedback.jsonl");
  try {
    symlinkSync(target, path);
    assert.throws(() => appendAgentFeedback(path, record("blocked")));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
