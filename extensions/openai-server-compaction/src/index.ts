/**
 * Main extension entrypoint.
 *
 * Wires together request patching, remote compaction, runtime state
 * reconstruction, session lifecycle cleanup, and provider override registration.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { AgentMessage } from "@earendil-works/pi-agent-core";
import * as PiAI from "@earendil-works/pi-ai";
import { openAICodexResponsesApi } from "@earendil-works/pi-ai/compat";
import { Effect } from "effect";
import { isRecord, loadConfig } from "./config.ts";
import {
  executeContinuationCompaction,
  RESPONSE_OUTPUT_ITEM_HOOK_VERSION,
  responsesCompactionStreamLayer,
  type CompactionRequestShape,
  type ContinuationCompactionStream,
} from "./continuation-compaction.ts";
import { streamOpenAIResponsesWithPhase2B } from "./custom-stream.ts";
import {
  applyPayloadPatch,
  applyRemoteHistoryPayloadPatch,
  extractAssistantResponseId,
  extractResponsesReasoningConfig,
  extractResponsesTextConfig,
  isOpenAICodexResponsesModel,
  looksLikeResponsesPayload,
  messageMatchesModel,
  modelKey,
  supportsPreviousResponseId,
  supportsRemoteCompactionModel,
  thinkingLevelToResponsesReasoning,
} from "./openai.ts";
import { releaseAllWsSessions, releaseWsSession } from "./openai-ws-stream.ts";
import {
  buildCompactionSummaryText,
  buildRemoteCompactionDetails,
  buildRemoteCompactionV2History,
  buildToolsPayload,
  callRemoteCompactionEndpoint,
  messageToResponseItems,
  messagesToResponseItems,
  normalizeResponseItemsForPrompt,
  reconstructRemoteCompactionStateFromBranch,
  withRemoteCompactionV2Feature,
} from "./remote-compaction.ts";
import {
  clearAllContinuationState,
  clearContinuationState,
  clearRemoteCompactionState,
  clearResponsesRequestShapeState,
  getContinuationState,
  getRemoteCompactionState,
  getResponsesRequestShapeState,
  setContinuationState,
  setRemoteCompactionState,
  setResponsesRequestShapeState,
} from "./state.ts";

type TargetModel = Parameters<typeof modelKey>[0];

type BranchEntry = {
  type: string;
  id: string;
  details?: unknown;
  message?: unknown;
  thinkingLevel?: unknown;
};

type SessionContextLike = {
  sessionManager: {
    getSessionId(): string;
    getBranch(): BranchEntry[];
  };
};

function getSessionId(ctx: SessionContextLike): string {
  return ctx.sessionManager.getSessionId();
}

function getBranchMessages(branchEntries: BranchEntry[]): AgentMessage[] {
  return branchEntries.flatMap((entry) =>
    entry.type === "message" && entry.message ? [entry.message as AgentMessage] : [],
  );
}

function getBranchMessageCount(branchEntries: BranchEntry[]): number {
  return branchEntries.filter((entry) => entry.type === "message" && Boolean(entry.message)).length;
}

function getBranchThinkingLevel(branchEntries: BranchEntry[]): string | undefined {
  for (let index = branchEntries.length - 1; index >= 0; index--) {
    const entry = branchEntries[index];
    if (entry?.type !== "thinking_level_change") continue;
    return typeof entry.thinkingLevel === "string" ? entry.thinkingLevel : undefined;
  }
  return undefined;
}

function getActiveToolDefinitions(pi: ExtensionAPI): ReturnType<ExtensionAPI["getAllTools"]> {
  const active = new Set(pi.getActiveTools());
  return pi.getAllTools().filter((tool) => active.has(tool.name));
}

function supportsResponseOutputItemHook(): boolean {
  return (PiAI as Record<string, unknown>).responseOutputItemHookVersion ===
    RESPONSE_OUTPUT_ITEM_HOOK_VERSION;
}

function requestShapeFromPayload(payload: Record<string, unknown>): CompactionRequestShape {
  return {
    ...(typeof payload.instructions === "string" ? { instructions: payload.instructions } : {}),
    ...(Array.isArray(payload.tools) ? { tools: structuredClone(payload.tools) } : {}),
    ...(typeof payload.parallel_tool_calls === "boolean"
      ? { parallelToolCalls: payload.parallel_tool_calls }
      : {}),
    ...(payload.tool_choice !== undefined ? { toolChoice: structuredClone(payload.tool_choice) } : {}),
    ...(isRecord(payload.reasoning) ? { reasoning: structuredClone(payload.reasoning) } : {}),
    ...(isRecord(payload.text) ? { text: structuredClone(payload.text) } : {}),
    ...(payload.service_tier !== undefined ? { serviceTier: payload.service_tier } : {}),
  };
}

function clearLiveContinuation(sessionId: string | undefined): void {
  clearContinuationState(sessionId);
  releaseWsSession(sessionId);
}

function clearSessionRuntimeState(sessionId: string | undefined): void {
  clearLiveContinuation(sessionId);
  clearRemoteCompactionState(sessionId);
  clearResponsesRequestShapeState(sessionId);
}

function syncRemoteState(ctx: SessionContextLike): void {
  const sessionId = getSessionId(ctx);
  const branchEntries = ctx.sessionManager.getBranch() as Array<{
    type: string;
    id: string;
    details?: unknown;
    message?: AgentMessage;
  }>;
  const state = reconstructRemoteCompactionStateFromBranch({ branchEntries });
  if (state) {
    setRemoteCompactionState(sessionId, state);
  } else {
    clearRemoteCompactionState(sessionId);
  }
}

function getMatchingRemoteState(
  sessionId: string,
  model: TargetModel | undefined,
): ReturnType<typeof getRemoteCompactionState> {
  if (!model) return undefined;
  const remoteState = getRemoteCompactionState(sessionId);
  return remoteState && remoteState.modelKey === modelKey(model) ? remoteState : undefined;
}

function extendRemoteHistoryIfCompatible(params: {
  sessionId: string;
  model: TargetModel | undefined;
  message: AgentMessage;
}): void {
  const remoteState = getMatchingRemoteState(params.sessionId, params.model);
  if (!remoteState || !params.model) return;
  if (params.message.role === "assistant" && !messageMatchesModel(params.message, params.model)) {
    return;
  }

  const items = messageToResponseItems(params.message);
  if (items.length === 0) return;

  setRemoteCompactionState(params.sessionId, {
    ...remoteState,
    explicitHistory: [...remoteState.explicitHistory, ...items],
  });
}

function maybeNotifyRequestFeatures(params: {
  notifiedModels: Set<string>;
  hasUI: boolean;
  notify: boolean;
  ui: { notify(message: string, level: "info" | "warning"): void };
  model: TargetModel;
  features: string[];
}): void {
  if (!params.notify || !params.hasUI || params.features.length === 0) return;

  const key = `${String(params.model.provider)}/${String(params.model.id)}`;
  const noticeKey = `${key}:${params.features.join(",")}`;
  if (params.notifiedModels.has(noticeKey)) return;

  params.notifiedModels.add(noticeKey);
  params.ui.notify(`OpenAI compaction active for ${key} (${params.features.join(", ")})`, "info");
}

export default function openaiServerCompactionExtension(pi: ExtensionAPI) {
  const notifiedModels = new Set<string>();

  pi.registerProvider("openai", {
    api: "openai-responses",
    streamSimple: streamOpenAIResponsesWithPhase2B,
  });

  pi.on("session_start", (_event, ctx) => {
    const sessionId = getSessionId(ctx);
    clearLiveContinuation(sessionId);
    clearResponsesRequestShapeState(sessionId);
    syncRemoteState(ctx);
  });

  const clearBeforeSessionChange = (_event: unknown, ctx: SessionContextLike): void => {
    clearSessionRuntimeState(getSessionId(ctx));
  };
  pi.on("session_before_switch", clearBeforeSessionChange);
  pi.on("session_before_fork", clearBeforeSessionChange);
  pi.on("session_before_tree", clearBeforeSessionChange);

  const syncAfterSessionChange = (_event: unknown, ctx: SessionContextLike): void => {
    clearLiveContinuation(getSessionId(ctx));
    syncRemoteState(ctx);
  };
  pi.on("session_tree", syncAfterSessionChange);
  pi.on("session_compact", syncAfterSessionChange);

  pi.on("model_select", (_event, ctx) => {
    clearLiveContinuation(getSessionId(ctx));
  });

  pi.on("session_shutdown", () => {
    clearAllContinuationState();
    releaseAllWsSessions();
  });

  pi.on("session_before_compact", async (event, ctx) => {
    const cfg = loadConfig(ctx.cwd);
    const model = ctx.model;
    if (!cfg.enabled || !model || !supportsRemoteCompactionModel(model)) return undefined;

    const auth = await ctx.modelRegistry.getApiKeyAndHeaders(model);
    if (!auth.ok || !auth.apiKey) return undefined;

    const activeToolDefinitions = getActiveToolDefinitions(pi);
    const tools = buildToolsPayload(activeToolDefinitions, pi.getActiveTools());
    const sessionId = getSessionId(ctx);
    const branchEntries = event.branchEntries as BranchEntry[];
    const remoteState = getMatchingRemoteState(sessionId, model);
    const observedRequestShape = getResponsesRequestShapeState(sessionId);
    const fullBranchMessages = getBranchMessages(branchEntries);
    const responseItems = remoteState
      ? remoteState.explicitHistory
      : messagesToResponseItems(fullBranchMessages);
    const promptResponseItems = normalizeResponseItemsForPrompt(responseItems, model);
    const thinkingLevel = pi.getThinkingLevel();
    const fallbackReasoning = model.reasoning
      ? thinkingLevelToResponsesReasoning(thinkingLevel ?? getBranchThinkingLevel(branchEntries))
      : undefined;
    const reasoning = observedRequestShape?.reasoning ?? fallbackReasoning;
    const text = observedRequestShape?.text;

    let remoteResult;
    try {
      if (isOpenAICodexResponsesModel(model) && supportsResponseOutputItemHook()) {
        const provider = openAICodexResponsesApi();
        const continuationResult = await Effect.runPromise(executeContinuationCompaction({
          model,
          context: {
            systemPrompt: ctx.getSystemPrompt(),
            messages: fullBranchMessages,
            tools: activeToolDefinitions,
          },
          streamOptions: {
            apiKey: auth.apiKey,
            headers: withRemoteCompactionV2Feature(auth.headers ?? {}),
            sessionId,
            signal: event.signal,
            transport: "websocket-cached",
            maxRetries: 2,
            ...(thinkingLevel && thinkingLevel !== "off"
              ? { reasoningEffort: thinkingLevel }
              : {}),
          },
          ...(remoteState ? { explicitPromptInput: promptResponseItems } : {}),
          requestShape: observedRequestShape,
        }).pipe(
          Effect.provide(
            responsesCompactionStreamLayer(provider.stream as ContinuationCompactionStream),
          ),
        ));
        remoteResult = {
          output: buildRemoteCompactionV2History(
            continuationResult.promptInput,
            continuationResult.compactionItem,
          ),
          usage: continuationResult.usage,
        };
      } else {
        remoteResult = await callRemoteCompactionEndpoint({
          model,
          apiKey: auth.apiKey,
          headers: auth.headers,
          sessionId,
          input: promptResponseItems,
          instructions: ctx.getSystemPrompt(),
          tools,
          parallelToolCalls: true,
          reasoning,
          text,
          signal: event.signal,
        });
      }
    } catch (error) {
      if (event.signal.aborted === false && ctx.hasUI) {
        const message = error instanceof Error ? error.message : String(error);
        ctx.ui.notify(`OpenAI remote compaction failed; falling back to default compaction. ${message}`, "warning");
      }
      return undefined;
    }

    const remoteDetails = buildRemoteCompactionDetails(
      model,
      remoteResult.output,
      remoteResult.usage,
    );

    return {
      compaction: {
        // Pi requires a summary string for its compaction entry. Keep only a
        // static marker; do not spend another model call on a local summary.
        summary: buildCompactionSummaryText(model),
        firstKeptEntryId: event.preparation.firstKeptEntryId,
        tokensBefore: event.preparation.tokensBefore,
        details: { remoteCompaction: remoteDetails },
      },
    };
  });

  pi.on("message_end", (event, ctx) => {
    const sessionId = getSessionId(ctx);
    const model = ctx.model;

    extendRemoteHistoryIfCompatible({
      sessionId,
      model,
      message: event.message,
    });

    const cfg = loadConfig(ctx.cwd);
    if (!cfg.enabled || !supportsPreviousResponseId(model, cfg)) return;
    if (!messageMatchesModel(event.message, model)) return;

    const responseId = extractAssistantResponseId(event.message);
    if (!responseId) return;

    setContinuationState(sessionId, {
      responseId,
      modelKey: modelKey(model),
      updatedAt: Date.now(),
      contextLength: getBranchMessageCount(ctx.sessionManager.getBranch() as BranchEntry[]),
    });
  });

  pi.on("before_provider_request", (event, ctx) => {
    const cfg = loadConfig(ctx.cwd);
    if (!cfg.enabled) return undefined;

    const model = ctx.model;
    if (!model || !isRecord(event.payload) || !looksLikeResponsesPayload(event.payload)) return undefined;

    const sessionId = getSessionId(ctx);
    setResponsesRequestShapeState(sessionId, {
      updatedAt: Date.now(),
      ...requestShapeFromPayload(event.payload),
      reasoning: extractResponsesReasoningConfig(event.payload),
      text: extractResponsesTextConfig(event.payload),
    });
    const remoteState = getMatchingRemoteState(sessionId, model);

    if (isOpenAICodexResponsesModel(model)) {
      if (!remoteState) return undefined;
      const payload = applyRemoteHistoryPayloadPatch({
        payload: event.payload,
        explicitHistory: normalizeResponseItemsForPrompt(remoteState.explicitHistory, model) as unknown[],
      });
      maybeNotifyRequestFeatures({
        notifiedModels,
        hasUI: ctx.hasUI,
        notify: cfg.notify,
        ui: ctx.ui,
        model,
        features: ["remote_compaction_history"],
      });
      return payload;
    }

    if (!supportsPreviousResponseId(model, cfg)) return undefined;

    const continuation = getContinuationState(sessionId);
    const previousResponseId =
      remoteState === undefined && continuation && continuation.modelKey === modelKey(model)
        ? continuation.responseId
        : undefined;

    const payload = applyPayloadPatch({
      payload: event.payload,
      model,
      cfg,
      previousResponseId,
    });

    const features = ["store=true", "context_management"];
    if (remoteState !== undefined) {
      features.push("remote_compaction_history");
    } else if (previousResponseId) {
      features.push("previous_response_id");
    }

    maybeNotifyRequestFeatures({
      notifiedModels,
      hasUI: ctx.hasUI,
      notify: cfg.notify,
      ui: ctx.ui,
      model,
      features,
    });

    return payload;
  });
}
