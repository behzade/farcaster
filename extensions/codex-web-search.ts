import { StringEnum, Type } from "@earendil-works/pi-ai";
import { Effect } from "effect";
import {
  DEFAULT_MAX_BYTES,
  DEFAULT_MAX_LINES,
  defineTool,
  type ExtensionAPI,
  type ExtensionContext,
  truncateHead,
} from "@earendil-works/pi-coding-agent";
import {
  buildCodexSearchRequest,
  extractCodexAccountId,
  parseCodexSearchResponse,
  redactCredential,
  type CodexSearchOptions,
} from "./codex-web-search-core.ts";

const CODEX_SEARCH_URL = "https://chatgpt.com/backend-api/codex/alpha/search";
const SEARCH_TIMEOUT_MS = 60_000;

const parameters = Type.Object({
  query: Type.String({ description: "Web search query" }),
  recency: Type.Optional(StringEnum(["day", "week", "month", "year"] as const)),
  domains: Type.Optional(Type.Array(Type.String(), { description: "Allowed domains; prefix exclusions with -" })),
});

const webSearchTool = defineTool({
  name: "web_search",
  label: "Codex Web Search",
  description: "Search the current web through Codex and return its cited response.",
  parameters,
  async execute(_toolCallId, params, signal, onUpdate, ctx) {
    const query = params.query.trim();
    if (!query) throw new Error("Web search query must not be empty");

    onUpdate?.({ content: [{ type: "text", text: `Searching: ${query}` }], details: { phase: "searching" } });
    const auth = await resolveCodexAuth(ctx);
    if (!auth) throw new Error("Codex Web Search requires an OpenAI Codex login. Use /login first.");

    const headers: Record<string, string> = {
      ...stringHeaders(auth.headers),
      Authorization: `Bearer ${auth.apiKey}`,
      "Content-Type": "application/json",
      originator: "pi",
    };
    const accountId = extractCodexAccountId(auth.apiKey);
    if (accountId) headers["chatgpt-account-id"] = accountId;

    const options: CodexSearchOptions = {
      recency: params.recency,
      domains: params.domains,
    };
    const combinedSignal = signal
      ? AbortSignal.any([signal, AbortSignal.timeout(SEARCH_TIMEOUT_MS)])
      : AbortSignal.timeout(SEARCH_TIMEOUT_MS);

    try {
      const { response, text } = await Effect.runPromise(Effect.tryPromise({
        try: async () => {
          const response = await fetch(CODEX_SEARCH_URL, {
            method: "POST",
            headers,
            body: JSON.stringify(buildCodexSearchRequest(
              query,
              ctx.sessionManager.getSessionId(),
              auth.model,
              options,
            )),
            signal: combinedSignal,
          });
          return { response, text: await response.text() };
        },
        catch: (error) => error instanceof Error ? error : new Error(String(error)),
      }), { signal: combinedSignal });
      if (!response.ok) {
        throw new Error(`Codex search error ${response.status}: ${redactCredential(text, auth.apiKey).slice(0, 500)}`);
      }

      const parsed = parseCodexSearchResponse(text);
      const truncation = truncateHead(parsed.output, {
        maxBytes: DEFAULT_MAX_BYTES,
        maxLines: DEFAULT_MAX_LINES,
      });
      return {
        content: [{
          type: "text",
          text: truncation.truncated
            ? `${truncation.content}\n\n[Codex search output truncated.]`
            : truncation.content,
        }],
        details: {
          model: auth.model,
          results: parsed.results,
          truncated: truncation.truncated,
        },
      };
    } catch (error) {
      const original = error instanceof Error ? error.message : String(error);
      const message = redactCredential(original, auth.apiKey);
      if (message === original) throw error;
      throw new Error(message);
    }
  },
});

export default function (pi: ExtensionAPI) {
  pi.registerTool(webSearchTool);
}

async function resolveCodexAuth(ctx: ExtensionContext) {
  const model = ctx.model?.provider === "openai-codex"
    ? ctx.model
    : ctx.modelRegistry.getAll().find((candidate) => candidate.provider === "openai-codex");
  if (!model) return undefined;
  try {
    const resolved = await ctx.modelRegistry.getApiKeyAndHeaders(model);
    if (!resolved.ok || !resolved.apiKey) return undefined;
    return {
      apiKey: resolved.apiKey,
      model: model.id,
      headers: resolved.headers ?? {},
    };
  } catch {
    return undefined;
  }
}

function stringHeaders(headers: Record<string, string | null>): Record<string, string> {
  return Object.fromEntries(Object.entries(headers).filter((entry): entry is [string, string] => entry[1] !== null));
}
