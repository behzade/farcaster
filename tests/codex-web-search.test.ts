import assert from "node:assert/strict";
import test from "node:test";
import {
  buildCodexSearchRequest,
  extractCodexAccountId,
  parseCodexSearchResponse,
  redactCredential,
} from "../extensions/codex-web-search-core.ts";

test("builds the direct Codex search request with bounded filters", () => {
  const request = buildCodexSearchRequest("current docs", "session-123", "gpt-test", {
    recency: "week",
    domains: ["https://docs.example.com/path", "-old.example.com", "invalid"],
  });

  assert.deepEqual(request, {
    id: "session-123",
    model: "gpt-test",
    input: "current docs",
    commands: {
      search_query: [{ q: "current docs", recency: 7, domains: ["docs.example.com"] }],
      response_length: "long",
    },
    settings: {
      allowed_callers: ["direct"],
      external_web_access: true,
      filters: {
        allowed_domains: ["docs.example.com"],
        blocked_domains: ["old.example.com"],
      },
    },
    max_output_tokens: 4_000,
  });
});

test("parses Codex output and preserves opaque result metadata", () => {
  const response = parseCodexSearchResponse(JSON.stringify({
    output: "Cited search result [1].",
    results: [{ type: "text_result", ref_id: "turn0search0", future_field: true }],
  }));

  assert.equal(response.output, "Cited search result [1].");
  assert.deepEqual(response.results, [
    { type: "text_result", ref_id: "turn0search0", future_field: true },
  ]);
  assert.throws(() => parseCodexSearchResponse("{}"), /returned no output/);
});

test("extracts Codex account IDs without exposing credentials", () => {
  const payload = Buffer.from(JSON.stringify({
    "https://api.openai.com/auth": { chatgpt_account_id: "account-123" },
  })).toString("base64url");
  const token = `header.${payload}.signature`;

  assert.equal(extractCodexAccountId(token), "account-123");
  assert.equal(redactCredential(`failed with ${token}`, token), "failed with [redacted]");
});
