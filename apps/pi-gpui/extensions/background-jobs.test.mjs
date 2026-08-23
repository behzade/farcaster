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

test("publishes background job starts, settlements, and removals", async () => {
  const { handlers, snapshots, ctx } = harness();
  await handlers.get("session_start")({}, ctx);
  await handlers.get("tool_execution_start")({
    toolCallId: "start",
    toolName: "background_job",
    args: { action: "start", name: "pi-server", command: "npm run dev" },
  }, ctx);
  await handlers.get("tool_execution_end")({
    toolCallId: "start",
    toolName: "background_job",
    result: { content: [{ type: "text", text: "started pi-server" }] },
    isError: false,
  }, ctx);
  await handlers.get("message_end")({
    message: {
      role: "custom",
      customType: "background-job-result",
      details: { name: "pi-server", state: "completed", exitCode: 0 },
    },
  }, ctx);

  assert.deepEqual(snapshots.at(-1), [{
    name: "pi-server",
    command: "npm run dev",
    state: "completed",
    exitCode: 0,
  }]);

  await handlers.get("tool_execution_start")({
    toolCallId: "stop",
    toolName: "background_job",
    args: { action: "stop", name: "pi-server" },
  }, ctx);
  await handlers.get("tool_execution_end")({
    toolCallId: "stop",
    toolName: "background_job",
    result: { content: [{ type: "text", text: "stopped pi-server" }] },
  }, ctx);
  assert.deepEqual(snapshots.at(-1), []);
});

test("removes a job when start is rejected", async () => {
  const { handlers, snapshots, ctx } = harness();
  await handlers.get("tool_execution_start")({
    toolCallId: "rejected",
    toolName: "background_job",
    args: { action: "start", name: "pi-test", command: "cargo test" },
  }, ctx);
  await handlers.get("tool_execution_end")({
    toolCallId: "rejected",
    toolName: "background_job",
    result: { content: [{ type: "text", text: "job already exists" }] },
    isError: true,
  }, ctx);

  assert.deepEqual(snapshots.at(-1), []);
});
