import assert from "node:assert/strict";
import test from "node:test";
import { executeHostScript } from "../extensions/lib/user-input-core.ts";

test("host script execution returns success output and runs exactly once", async () => {
  const calls: Array<{ command: string; cwd: string }> = [];
  const result = await executeHostScript({
    async exec(command, cwd, { onData }) {
      calls.push({ command, cwd });
      onData(Buffer.from("done\n"));
      return { exitCode: 0 };
    },
  }, "printf done", "/project");

  assert.deepEqual(calls, [{ command: "printf done", cwd: "/project" }]);
  assert.deepEqual(result, {
    status: "success",
    exitCode: 0,
    output: "done",
    truncated: false,
  });
});

test("host script execution preserves non-zero exit output as failure", async () => {
  const result = await executeHostScript({
    async exec(_command, _cwd, { onData }) {
      onData(Buffer.from("bad input\n"));
      return { exitCode: 7 };
    },
  }, "false", "/project");

  assert.equal(result.status, "failure");
  assert.equal(result.exitCode, 7);
  assert.equal(result.output, "bad input");
});

test("host script execution reports launch failures", async () => {
  const result = await executeHostScript({
    async exec() {
      throw new Error("shell unavailable");
    },
  }, "echo unreachable", "/project");

  assert.deepEqual(result, {
    status: "failure",
    exitCode: null,
    output: "shell unavailable",
    truncated: false,
  });
});

test("host script output is bounded while retaining the tail", async () => {
  const result = await executeHostScript({
    async exec(_command, _cwd, { onData }) {
      onData(Buffer.from("abcdefgh"));
      onData(Buffer.from("ijkl"));
      return { exitCode: 0 };
    },
  }, "verbose", "/project", undefined, 8);

  assert.equal(result.status, "success");
  assert.equal(result.truncated, true);
  assert.equal(result.output, "[output truncated to last 8 bytes]\nefghijkl");
});
