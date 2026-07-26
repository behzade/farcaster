import assert from "node:assert/strict";
import test from "node:test";
import {
  evaluateCommand,
  overallDecision,
  parseShellCommands,
  type CommandPolicy,
} from "../extensions/guardian/preflight/command-policy.ts";
import { NotificationCoalescer, osc9Sequence, preview } from "../extensions/lib/notification-core.ts";

const policy: CommandPolicy = {
  defaultDecision: "allow",
  rules: [
    { id: "git-add", pattern: ["git", "add"], decision: "allow" },
    { id: "git-add-all", pattern: ["git", "add", "."], decision: "prompt" },
    { id: "rm-rf", pattern: ["rm"], allFlags: ["r", "f"], decision: "prompt" },
    { id: "old-filter", pattern: ["git", "filter-branch"], decision: "forbid" },
  ],
};

test("parses quoted arguments and each shell command", () => {
  assert.deepEqual(parseShellCommands(`printf '%s' 'a b'; git add "file name" && cargo test`), [
    { argv: ["printf", "%s", "a b"] },
    { argv: ["git", "add", "file name"] },
    { argv: ["cargo", "test"] },
  ]);
});

test("the most specific prefix decides", () => {
  assert.equal(overallDecision(evaluateCommand("git add .", policy))?.decision, "prompt");
  assert.equal(overallDecision(evaluateCommand("git add one.txt", policy))?.decision, "allow");
});

test("clustered flags meet parsed flag rules", () => {
  assert.equal(overallDecision(evaluateCommand("rm -rf build", policy))?.ruleId, "rm-rf");
  assert.equal(overallDecision(evaluateCommand("rm -fr build", policy))?.ruleId, "rm-rf");
  assert.equal(overallDecision(evaluateCommand("rm -r build", policy))?.decision, "allow");
});

test("the strictest command in a compound command wins", () => {
  const result = overallDecision(evaluateCommand("echo ready && git filter-branch -- --all", policy));
  assert.equal(result?.decision, "forbid");
  assert.equal(result?.ruleId, "old-filter");
});

test("environment assignments and wrappers do not hide commands", () => {
  assert.equal(overallDecision(evaluateCommand("A=1 env B=2 git add .", policy))?.ruleId, "git-add-all");
});

test("shell evaluation does not hide nested commands", () => {
  assert.equal(overallDecision(evaluateCommand(`bash -c 'git filter-branch -- --all'`, policy))?.decision, "forbid");
  assert.equal(overallDecision(evaluateCommand("echo `git add .`", policy))?.ruleId, "git-add-all");
});

test("background jobs do not hide the command from policy", () => {
  const helper = "/home/me/.pi/agent/skills/background-jobs/scripts/job.sh";
  assert.equal(
    overallDecision(evaluateCommand(`bash ${helper} start pi-build /repo 'rm -rf build'`, policy))?.ruleId,
    "rm-rf",
  );
  assert.equal(
    overallDecision(evaluateCommand(`${helper} start pi-rewrite /repo 'git filter-branch -- --all'`, policy))?.decision,
    "forbid",
  );
});

test("an approval flow outranks a pending completion notice", () => {
  const decision = overallDecision(evaluateCommand("git add .", policy));
  assert.equal(decision?.decision, "prompt");

  const notices = new NotificationCoalescer<{ type: string; priority: number }>();
  notices.push({ type: "agent-turn-complete", priority: 1 });
  notices.push({ type: `approval-${decision?.ruleId}`, priority: 2 });
  assert.deepEqual(notices.take(), { type: "approval-git-add-all", priority: 2 });
  assert.equal(notices.take(), undefined);
});

test("notification previews are short and safe for OSC 9", () => {
  assert.equal(preview("a\n  b"), "a b");
  assert.equal(osc9Sequence("hello\u001b]9;bad\u0007", false), "\u001b]9;hello ]9;bad \u0007");
  assert.equal(osc9Sequence("done", true), "\u001bPtmux;\u001b\u001b]9;done\u0007\u001b\\");
});
