import { createHash } from "node:crypto";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export interface PromptInspectorTool {
  name: string;
  description: string;
  parameters: unknown;
}

export interface PromptReportInput {
  systemPrompt: string;
  activeToolNames: readonly string[];
  tools: readonly PromptInspectorTool[];
}

export interface PromptSchemaContributor {
  name: string;
  characters: number;
}

export interface PromptReport {
  systemPrompt: string;
  systemPromptCharacters: number;
  roughSystemPromptTokens: number;
  systemPromptSha256: string;
  activeToolNames: string[];
  activeDefinitions: PromptInspectorTool[];
  serializedActiveDefinitions: string;
  activeDefinitionCharacters: number;
  roughActiveDefinitionTokens: number;
  activeDefinitionsSha256: string;
  largestSchemaContributors: PromptSchemaContributor[];
}

const roughTokens = (characters: number): number => Math.ceil(characters / 4);
const sha256 = (value: string): string => createHash("sha256").update(value).digest("hex");

/** Build deterministic, pre-provider prompt and schema measurements. */
export function buildPromptReport(input: PromptReportInput): PromptReport {
  const byName = new Map(input.tools.map((tool) => [tool.name, tool]));
  const activeDefinitions = input.activeToolNames.flatMap((name) => {
    const tool = byName.get(name);
    return tool === undefined
      ? []
      : [{ name: tool.name, description: tool.description, parameters: tool.parameters }];
  });
  const serializedActiveDefinitions = JSON.stringify(activeDefinitions);
  const largestSchemaContributors = activeDefinitions
    .map((tool) => ({ name: tool.name, characters: JSON.stringify(tool).length }))
    .sort((left, right) => right.characters - left.characters || left.name.localeCompare(right.name));

  return {
    systemPrompt: input.systemPrompt,
    systemPromptCharacters: input.systemPrompt.length,
    roughSystemPromptTokens: roughTokens(input.systemPrompt.length),
    systemPromptSha256: sha256(input.systemPrompt),
    activeToolNames: [...input.activeToolNames],
    activeDefinitions,
    serializedActiveDefinitions,
    activeDefinitionCharacters: serializedActiveDefinitions.length,
    roughActiveDefinitionTokens: roughTokens(serializedActiveDefinitions.length),
    activeDefinitionsSha256: sha256(serializedActiveDefinitions),
    largestSchemaContributors,
  };
}

/** Render a human-only report. Estimates are deliberately labeled as rough. */
export function formatPromptReport(report: PromptReport, full: boolean): string {
  const contributors = report.largestSchemaContributors.length === 0
    ? "(none)"
    : report.largestSchemaContributors
      .map((entry) => `- ${entry.name}: ${entry.characters} chars`)
      .join("\n");
  const summary = `# Prompt report

System prompt: ${report.systemPromptCharacters} chars (~${report.roughSystemPromptTokens} tokens at 4 chars/token)
System SHA-256: ${report.systemPromptSha256}
Active tools (${report.activeToolNames.length}): ${report.activeToolNames.join(", ") || "(none)"}
Pre-provider active definitions: ${report.activeDefinitionCharacters} chars (~${report.roughActiveDefinitionTokens} tokens at 4 chars/token)
Definitions SHA-256: ${report.activeDefinitionsSha256}

Largest schema contributors:
${contributors}

Estimates are rough character heuristics, not provider token counts or billing data.`;
  if (!full) return summary;
  return `${summary}

# Exact effective system prompt

${report.systemPrompt}

# Exact serialized active definitions (pre-provider)

${report.serializedActiveDefinitions}`;
}

export default function promptInspector(pi: ExtensionAPI) {
  pi.registerCommand("prompt-report", {
    description: "Inspect model-visible prompt and active schema size without a model call",
    handler: async (args, ctx) => {
      if (!ctx.hasUI) return;
      const report = buildPromptReport({
        systemPrompt: ctx.getSystemPrompt(),
        activeToolNames: pi.getActiveTools(),
        tools: pi.getAllTools(),
      });
      await ctx.ui.editor("Prompt report (changes are discarded)", formatPromptReport(report, args.trim() === "full"));
    },
  });
}
