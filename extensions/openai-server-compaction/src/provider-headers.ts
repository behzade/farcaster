import type { ProviderHeaders } from "@earendil-works/pi-ai";

const REMOTE_COMPACTION_V2_FEATURE = "remote_compaction_v2";

export function mergeProviderHeaders(...layers: ProviderHeaders[]): ProviderHeaders {
  const merged = new Map<string, { name: string; value: string | null }>();
  for (const layer of layers) {
    for (const [name, value] of Object.entries(layer)) {
      merged.set(name.toLowerCase(), { name, value });
    }
  }
  return Object.fromEntries(
    [...merged.values()].map(({ name, value }) => [name, value]),
  );
}

export function resolveProviderHeaders(...layers: ProviderHeaders[]): Record<string, string> {
  return Object.fromEntries(
    Object.entries(mergeProviderHeaders(...layers)).filter(
      (entry): entry is [string, string] => entry[1] !== null,
    ),
  );
}

export function withRemoteCompactionV2Feature(headers: ProviderHeaders): ProviderHeaders {
  const configuredValue = Object.entries(headers)
    .find(([name]) => name.toLowerCase() === "x-codex-beta-features")?.[1];
  const configuredFeatures = typeof configuredValue === "string"
    ? configuredValue.split(",").map((feature) => feature.trim()).filter(Boolean)
    : [];
  const headersWithoutFeature = Object.fromEntries(
    Object.entries(headers).filter(([name]) => name.toLowerCase() !== "x-codex-beta-features"),
  ) as ProviderHeaders;
  const features = [...new Set([...configuredFeatures, REMOTE_COMPACTION_V2_FEATURE])];
  return {
    ...headersWithoutFeature,
    "x-codex-beta-features": features.join(","),
  };
}
