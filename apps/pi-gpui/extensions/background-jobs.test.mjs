import assert from "node:assert/strict";
import test from "node:test";

import backgroundJobs, {
  BACKGROUND_JOBS_STATUS_KEY,
} from "./companion/background-jobs.ts";

function harness() {
  const handlers = new Map();
  const snapshots = [];
  backgroundJobs({
    on(event, callback) {
      handlers.set(event, callback);
    },
  });
  const ctx = {
    ui: {
      setStatus(key, text) {
        assert.equal(key, BACKGROUND_JOBS_STATUS_KEY);
        snapshots.push(JSON.parse(text));
      },
    },
  };
  return { handlers, snapshots, ctx };
}

async function startBash(handlers, ctx, toolCallId = "bash-call") {
  await handlers.get("tool_execution_start")({
    toolCallId,
    toolName: "bash",
    args: { command: "cargo check" },
  }, ctx);
}

async function finishDetachedBash(handlers, ctx, id = "pi-process", toolCallId = "bash-call") {
  await handlers.get("tool_execution_end")({
    toolCallId,
    toolName: "bash",
    result: { details: { id, state: "running" } },
    isError: false,
  }, ctx);
}

test("publishes detached bash processes and their settlements", async () => {
  const { handlers, snapshots, ctx } = harness();
  await handlers.get("session_start")({}, ctx);
  await startBash(handlers, ctx);
  await finishDetachedBash(handlers, ctx);

  assert.deepEqual(snapshots.at(-1), [{
    name: "pi-process",
    command: "cargo check",
    state: "running",
  }]);

  await handlers.get("message_end")({
    message: {
      role: "custom",
      customType: "process-session-result",
      details: { id: "pi-process", state: "exited", exitCode: 7 },
    },
  }, ctx);

  assert.deepEqual(snapshots.at(-1), [{
    name: "pi-process",
    command: "cargo check",
    state: "exited",
    exitCode: 7,
  }]);
});

test("applies completion delivered before the detached bash result", async () => {
  const { handlers, snapshots, ctx } = harness();
  await handlers.get("session_start")({}, ctx);
  await startBash(handlers, ctx);
  await handlers.get("message_end")({
    message: {
      role: "custom",
      customType: "process-session-result",
      details: { id: "pi-process", state: "completed", exitCode: 0 },
    },
  }, ctx);
  await finishDetachedBash(handlers, ctx);

  assert.deepEqual(snapshots.at(-1), [{
    name: "pi-process",
    command: "cargo check",
    state: "completed",
    exitCode: 0,
  }]);
});

test("does not publish short or failed bash calls", async () => {
  const { handlers, snapshots, ctx } = harness();
  await handlers.get("session_start")({}, ctx);

  for (const [toolCallId, isError, details] of [
    ["short", false, undefined],
    ["failed", true, { id: "pi-failed", state: "running" }],
  ]) {
    await handlers.get("tool_execution_start")({
      toolCallId,
      toolName: "bash",
      args: { command: "printf ok" },
    }, ctx);
    await handlers.get("tool_execution_end")({
      toolCallId,
      toolName: "bash",
      result: { details },
      isError,
    }, ctx);
  }

  assert.deepEqual(snapshots, [[]]);
});
