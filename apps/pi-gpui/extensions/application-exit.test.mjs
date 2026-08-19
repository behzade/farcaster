import assert from "node:assert/strict";
import test from "node:test";

import applicationExit, { APPLICATION_EXIT_MESSAGE } from "./companion/application-exit.ts";

function shutdownHandler() {
  let handler;
  applicationExit({
    on(event, callback) {
      if (event === "session_shutdown") handler = callback;
    },
  });
  assert.equal(typeof handler, "function");
  return handler;
}

function context(idle, appended) {
  return {
    isIdle: () => idle,
    sessionManager: {
      appendCustomMessageEntry(...args) {
        appended.push(args);
      },
    },
  };
}

test("records an active turn interrupted by application exit", async () => {
  const appended = [];
  await shutdownHandler()({ reason: "quit" }, context(false, appended));

  assert.deepEqual(appended, [[
    "pi-gpui-application-exit",
    APPLICATION_EXIT_MESSAGE,
    true,
    { reason: "application-exit" },
  ]]);
});

test("does not record idle or non-exit shutdowns", async () => {
  const appended = [];
  const handler = shutdownHandler();
  await handler({ reason: "quit" }, context(true, appended));
  await handler({ reason: "reload" }, context(false, appended));

  assert.deepEqual(appended, []);
});
