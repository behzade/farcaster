import assert from "node:assert/strict";
import test from "node:test";
import {
  normalizeMoonshotSchema,
  normalizeMoonshotToolPayload,
} from "../extensions/kimi-tool-schema.ts";

test("Moonshot schemas place a shared type inside anyOf branches", () => {
  const schema = {
    type: "object",
    properties: {
      action: { type: "string" },
      path: { type: "string" },
    },
    anyOf: [
      { required: ["action"] },
      { required: ["path"] },
    ],
  };

  assert.deepEqual(normalizeMoonshotSchema(schema), {
    properties: schema.properties,
    anyOf: [
      { type: "object", required: ["action"] },
      { type: "object", required: ["path"] },
    ],
  });
  assert.equal(schema.type, "object");
});

test("only function tool parameters are rewritten", () => {
  const untouched = { model: "other", messages: [] };
  assert.equal(normalizeMoonshotToolPayload(untouched), untouched);

  assert.deepEqual(normalizeMoonshotToolPayload({
    model: "kimi-k3",
    tools: [{
      type: "function",
      function: {
        name: "background_job",
        parameters: {
          type: "object",
          anyOf: [{ required: ["action"] }],
        },
      },
    }],
  }), {
    model: "kimi-k3",
    tools: [{
      type: "function",
      function: {
        name: "background_job",
        parameters: {
          anyOf: [{ type: "object", required: ["action"] }],
        },
      },
    }],
  });
});
