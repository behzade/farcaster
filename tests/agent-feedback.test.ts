import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, statSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import test from "node:test";
import {
  appendAgentFeedback,
  type AgentFeedbackRecord,
} from "../extensions/lib/agent-feedback.ts";

const installedSubagents = process.env.PI_SUBAGENTS_PACKAGE;

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

test("packaged feedback disables ambient child extensions", () => {
  const expression = readFileSync(
    new URL("../nix/pi-subagents.nix", import.meta.url),
    "utf8",
  );
  assert.match(
    expression,
    /extensions:\nsubagentOnlyExtensions: \$out\/agent-feedback\/index\.ts/,
  );
});

test("installed feedback extension loads once in an isolated child", {
  skip: installedSubagents ? false : "PI_SUBAGENTS_PACKAGE is set by the Nix integration check",
}, async () => {
  const reviewer = readFileSync(join(installedSubagents!, "agents", "reviewer.md"), "utf8");
  const frontmatterModule = await import(
    pathToFileURL(join(installedSubagents!, "src", "agents", "frontmatter.ts")).href
  ) as {
    parseFrontmatter(content: string): { frontmatter: Record<string, string> };
    parseFrontmatterList(value: string | undefined): string[] | undefined;
  };
  const launchModule = await import(
    pathToFileURL(join(installedSubagents!, "src", "runs", "shared", "pi-args.ts")).href
  ) as {
    resolvePiLaunchToolPlan(input: Record<string, unknown>): {
      disableAmbientExtensions: boolean;
      extensionArgs: string[];
    };
  };
  const { frontmatter } = frontmatterModule.parseFrontmatter(reviewer);
  const extensions = frontmatterModule.parseFrontmatterList(frontmatter.extensions);
  const subagentOnlyExtensions = frontmatterModule.parseFrontmatterList(
    frontmatter.subagentOnlyExtensions,
  );
  const plan = launchModule.resolvePiLaunchToolPlan({
    cwd: process.cwd(),
    tools: frontmatterModule.parseFrontmatterList(frontmatter.tools),
    extensions,
    subagentOnlyExtensions,
  });

  assert.deepEqual(extensions, []);
  assert.equal(plan.disableAmbientExtensions, true);
  assert.equal(
    plan.extensionArgs.filter((path) => path.endsWith("/agent-feedback/index.ts")).length,
    1,
  );
});
