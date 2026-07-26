import assert from "node:assert/strict";
import test from "node:test";
import { NotificationCoalescer, osc9Sequence, preview } from "../extensions/lib/notification-core.ts";

test("an approval request outranks a pending completion notice", () => {
  const notices = new NotificationCoalescer<{ type: string; priority: number }>();
  notices.push({ type: "agent-turn-complete", priority: 1 });
  notices.push({ type: "io-approval", priority: 2 });
  assert.deepEqual(notices.take(), { type: "io-approval", priority: 2 });
  assert.equal(notices.take(), undefined);
});

test("notification previews are short and safe for OSC 9", () => {
  assert.equal(preview("a\n  b"), "a b");
  assert.equal(osc9Sequence("hello\u001b]9;bad\u0007", false), "\u001b]9;hello ]9;bad \u0007");
  assert.equal(osc9Sequence("done", true), "\u001bPtmux;\u001b\u001b]9;done\u0007\u001b\\");
});
