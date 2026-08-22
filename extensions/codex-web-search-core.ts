export interface CodexSearchOptions {
  recency?: "day" | "week" | "month" | "year";
  domains?: string[];
}

export interface CodexSearchResponse {
  output: string;
  results?: unknown[];
}

const RECENCY_DAYS = {
  day: 1,
  week: 7,
  month: 30,
  year: 365,
} as const;

export function buildCodexSearchRequest(
  query: string,
  sessionId: string,
  model: string,
  options: CodexSearchOptions,
): Record<string, unknown> {
  const filters = normalizeDomainFilters(options.domains);
  return {
    id: sessionId,
    model,
    input: query,
    commands: {
      search_query: [{
        q: query,
        ...(options.recency ? { recency: RECENCY_DAYS[options.recency] } : {}),
        ...(filters.allowedDomains ? { domains: filters.allowedDomains } : {}),
      }],
      response_length: "long",
    },
    settings: {
      allowed_callers: ["direct"],
      external_web_access: true,
      ...(filters.allowedDomains || filters.blockedDomains ? {
        filters: {
          ...(filters.allowedDomains ? { allowed_domains: filters.allowedDomains } : {}),
          ...(filters.blockedDomains ? { blocked_domains: filters.blockedDomains } : {}),
        },
      } : {}),
    },
    max_output_tokens: 4_000,
  };
}

export function parseCodexSearchResponse(text: string): CodexSearchResponse {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    throw new Error(`Codex search returned invalid JSON: ${error instanceof Error ? error.message : String(error)}`);
  }
  if (!isRecord(value) || typeof value.output !== "string" || !value.output.trim()) {
    throw new Error("Codex search returned no output");
  }
  return {
    output: value.output.trim(),
    ...(Array.isArray(value.results) ? { results: value.results } : {}),
  };
}

export function extractCodexAccountId(token: string): string | undefined {
  const auth = decodeJwtPayload(token)?.["https://api.openai.com/auth"];
  if (!isRecord(auth)) return undefined;
  const accountId = auth.chatgpt_account_id;
  return typeof accountId === "string" && accountId.trim() ? accountId.trim() : undefined;
}

export function redactCredential(text: string, credential: string): string {
  return credential ? text.split(credential).join("[redacted]") : text;
}

function normalizeDomainFilters(values: string[] | undefined): {
  allowedDomains?: string[];
  blockedDomains?: string[];
} {
  const allowedDomains: string[] = [];
  const blockedDomains: string[] = [];
  for (const raw of values ?? []) {
    const blocked = raw.trim().startsWith("-");
    const domain = normalizeDomain(raw);
    if (!domain) continue;
    const target = blocked ? blockedDomains : allowedDomains;
    if (!target.includes(domain)) target.push(domain);
  }
  return {
    ...(allowedDomains.length > 0 ? { allowedDomains: allowedDomains.slice(0, 100) } : {}),
    ...(blockedDomains.length > 0 ? { blockedDomains: blockedDomains.slice(0, 100) } : {}),
  };
}

function normalizeDomain(raw: string): string | undefined {
  let value = raw.trim().toLowerCase();
  if (value.startsWith("-")) value = value.slice(1).trim();
  if (!value) return undefined;
  try {
    value = new URL(value.includes("://") ? value : `https://${value}`).hostname;
  } catch {
    return undefined;
  }
  value = value.replace(/^\.+|\.+$/g, "");
  return /^[a-z0-9][a-z0-9.-]*\.[a-z]{2,}$/i.test(value) ? value : undefined;
}

function decodeJwtPayload(token: string): Record<string, unknown> | undefined {
  const encoded = token.split(".")[1];
  if (!encoded) return undefined;
  try {
    const padded = encoded.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(encoded.length / 4) * 4, "=");
    const value: unknown = JSON.parse(Buffer.from(padded, "base64").toString("utf8"));
    return isRecord(value) ? value : undefined;
  } catch {
    return undefined;
  }
}

function isRecord(value: unknown): value is Record<string, any> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}
