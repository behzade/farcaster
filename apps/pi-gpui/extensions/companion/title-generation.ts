import { uuidv7 } from "@earendil-works/pi-ai";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

const DEFAULT_MODEL = "openai-codex/gpt-5.6-luna";
const MAX_TITLE_CHARS = 80;
const MAX_TITLE_WORDS = 12;

function configuredModel(): [string, string] {
  const selector = process.env.PI_GUI_TITLE_MODEL?.trim() || DEFAULT_MODEL;
  const separator = selector.indexOf("/");
  if (separator <= 0 || separator === selector.length - 1) {
    throw new Error(`invalid PI_GUI_TITLE_MODEL selector: ${selector}`);
  }
  return [selector.slice(0, separator), selector.slice(separator + 1)];
}

function normalize(value: string): string | undefined {
  const firstLine = value.split("\n").find((line) => line.trim())?.trim();
  if (!firstLine) return undefined;
  const unquoted = firstLine
    .replace(/^["`]+|["`]+$/g, "")
    .trim()
    .replace(/[.:;]+$/g, "");
  const words = unquoted.split(/\s+/).slice(0, MAX_TITLE_WORDS).join(" ");
  const title = [...words].slice(0, MAX_TITLE_CHARS).join("").trim();
  return title || undefined;
}

async function generateTitle(
  pi: ExtensionAPI,
  prompt: string,
  ctx: ExtensionContext,
  signal: AbortSignal,
): Promise<void> {
  const [provider, modelId] = configuredModel();
  const model = ctx.modelRegistry.find(provider, modelId);
  if (!model || !ctx.modelRegistry.hasConfiguredAuth(model)) return;

  const response = await ctx.modelRegistry.complete(
    model,
    {
      systemPrompt:
        "Create a concise semantic title for this coding session. Return only the title, without quotes, markdown, or punctuation at the end. Use at most 12 words.",
      messages: [
        {
          role: "user",
          content: [{ type: "text", text: `User request:\n${prompt.trim()}` }],
          timestamp: Date.now(),
        },
      ],
    },
    {
      maxTokens: 80,
      cacheRetention: "none",
      sessionId: uuidv7(),
      signal,
    },
  );
  const title = normalize(
    response.content
      .filter((part): part is { type: "text"; text: string } => part.type === "text")
      .map((part) => part.text)
      .join("\n"),
  );
  if (!signal.aborted && title && !pi.getSessionName()) pi.setSessionName(title);
}

export default function titleGeneration(pi: ExtensionAPI): void {
  let attempted = false;
  const controller = new AbortController();

  pi.on("before_agent_start", async (event, ctx) => {
    if (attempted || pi.getSessionName() || !event.prompt.trim()) return;
    attempted = true;
    try {
      await generateTitle(pi, event.prompt, ctx, controller.signal);
    } catch (error) {
      if (!controller.signal.aborted) {
        console.error(`Automatic session title generation failed: ${String(error)}`);
      }
    }
  });

  pi.on("session_shutdown", () => {
    controller.abort();
  });
}
