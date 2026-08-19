import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

type JsonObject = Record<string, unknown>;

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Move a shared type into anyOf branches without changing accepted values. */
export function normalizeMoonshotSchema(schema: unknown): unknown {
  if (Array.isArray(schema)) return schema.map(normalizeMoonshotSchema);
  if (!isObject(schema)) return schema;

  const normalized = Object.fromEntries(
    Object.entries(schema).map(([key, value]) => [key, normalizeMoonshotSchema(value)]),
  );
  if (normalized.type !== undefined && Array.isArray(normalized.anyOf)) {
    const type = normalized.type;
    normalized.anyOf = normalized.anyOf.map((variant) =>
      isObject(variant) && variant.type === undefined ? { type, ...variant } : variant,
    );
    delete normalized.type;
  }
  return normalized;
}

export function normalizeMoonshotToolPayload(payload: unknown): unknown {
  if (!isObject(payload) || !Array.isArray(payload.tools)) return payload;
  let changed = false;
  const tools = payload.tools.map((tool) => {
    if (!isObject(tool) || !isObject(tool.function) || tool.function.parameters === undefined) {
      return tool;
    }
    changed = true;
    return {
      ...tool,
      function: {
        ...tool.function,
        parameters: normalizeMoonshotSchema(tool.function.parameters),
      },
    };
  });
  return changed ? { ...payload, tools } : payload;
}

function needsMoonshotSchema(provider: string | undefined, model: string | undefined): boolean {
  return provider === "opencode-go" && model?.toLowerCase().includes("kimi-") === true;
}

export default function kimiToolSchema(pi: ExtensionAPI): void {
  pi.on("before_provider_request", (event, ctx) => {
    if (!needsMoonshotSchema(ctx.model?.provider, ctx.model?.id)) return;
    return normalizeMoonshotToolPayload(event.payload);
  });
}
