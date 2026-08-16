/**
 * Custom OpenAI Responses streaming implementation.
 *
 * Bridges Pi's stream API to OpenAI's Responses WebSocket transport, while also
 * handling HTTP fallback, incremental continuation, and remote-compaction-aware
 * input replay.
 */
import { randomUUID } from "node:crypto";
import { Cause, Context as EffectContext, Effect, Exit, type Fiber, Layer, ManagedRuntime, Queue, Schema, Semaphore, Stream } from "effect";
import {
  calculateCost,
  createAssistantMessageEventStream,
  type AssistantMessage,
  type AssistantMessageEvent,
  type AssistantMessageEventStream,
  type Context,
  type Message,
  type Model,
  type SimpleStreamOptions,
  type StopReason,
  type TextContent,
  type ToolCall,
  type Usage,
  type StreamFunction,
} from "@earendil-works/pi-ai";
import { streamSimpleOpenAIResponses } from "@earendil-works/pi-ai/compat";
import { loadConfig } from "./config.ts";
import {
  isDirectOpenAIResponsesModel,
  modelKey,
  thinkingLevelToResponsesReasoning,
} from "./openai.ts";
import {
  type ContentPart,
  type FunctionToolDefinition,
  type InputItem,
  isResponseObject,
  OpenAIWebSocketManager,
  type OpenAIResponsesAssistantPhase,
  type OpenAIWebSocketEvent,
  type OpenAIWebSocketManagerOptions,
  type ResponseObject,
} from "./openai-ws-connection.ts";
import {
  buildAssistantMessage,
  buildAssistantMessageWithZeroUsage,
  buildStreamErrorAssistantMessage,
} from "./stream-message-shared.ts";
import {
  buildCodexWebSocketHeaders,
  normalizeResponseItemsForPrompt,
} from "./remote-compaction.ts";
import {
  getContinuationState,
  getRemoteCompactionState,
  setContinuationState,
} from "./state.ts";

type WsSession = {
  manager: OpenAIWebSocketManager;
  modelKey: string;
  lastContextLength: number;
  lastRequestKey?: string;
  warmUpAttempted: boolean;
  broken: boolean;
};

interface WsSessionRegistryShape {
  readonly sessions: Map<string, WsSession>;
  readonly locks: Map<string, Semaphore.Semaphore>;
}

class WsSessionRegistry extends EffectContext.Service<WsSessionRegistry, WsSessionRegistryShape>()(
  "pi-openai-server-compaction/WsSessionRegistry",
) {}

const wsSessionRegistryLayer = Layer.sync(WsSessionRegistry, () => ({
  sessions: new Map(),
  locks: new Map(),
}));
const openAIWebSocketRuntime = ManagedRuntime.make(wsSessionRegistryLayer);
const activeStreamFibers = new Set<Fiber.Fiber<void, never>>();
const WARM_UP_TIMEOUT_MS = 8000;

type AssistantMessageEventStreamLike = {
  push(event: AssistantMessageEvent): void;
  end(result?: AssistantMessage): void;
  result(): Promise<AssistantMessage>;
  [Symbol.asyncIterator](): AsyncIterator<AssistantMessageEvent>;
};

type AnyMessage = Message & {
  role: string;
  content: unknown;
  toolCallId?: unknown;
  toolUseId?: unknown;
  phase?: unknown;
};

type AssistantMessageWithPhase = AssistantMessage & { phase?: OpenAIResponsesAssistantPhase };
type ReplayModelInfo = { input?: ReadonlyArray<string> };
type WsTransport = "sse" | "websocket" | "auto";
type ResponsesModel = Model<"openai-responses">;
type ModelDescriptor = { api: string; provider: string; id: string };
type WsOptions = SimpleStreamOptions & {
  openaiWsWarmup?: unknown;
  topP?: number;
  toolChoice?: unknown;
  serviceTier?: "auto" | "default" | "flex" | "priority";
  reasoningSummary?: "auto" | "concise" | "detailed" | null;
  text?: Record<string, unknown>;
};

function applyServiceTierPricing(
  usage: Usage,
  modelInfo: ModelDescriptor,
  serviceTier: "auto" | "default" | "flex" | "priority" | undefined,
): void {
  const priorityMultiplier = modelInfo.id === "gpt-5.5" ? 2.5 : 2;
  const multiplier = serviceTier === "flex" ? 0.5 : serviceTier === "priority" ? priorityMultiplier : 1;
  if (multiplier === 1) return;
  usage.cost.input *= multiplier;
  usage.cost.output *= multiplier;
  usage.cost.cacheRead *= multiplier;
  usage.cost.cacheWrite *= multiplier;
  usage.cost.total = usage.cost.input + usage.cost.output + usage.cost.cacheRead + usage.cost.cacheWrite;
}

function extractCacheWriteTokens(response: ResponseObject): number {
  const details = response.usage?.input_tokens_details as
    | { cache_creation_tokens?: unknown; cache_write_tokens?: unknown }
    | undefined;
  if (typeof details?.cache_creation_tokens === "number" && Number.isFinite(details.cache_creation_tokens)) {
    return details.cache_creation_tokens;
  }
  return typeof details?.cache_write_tokens === "number" && Number.isFinite(details.cache_write_tokens)
    ? details.cache_write_tokens
    : 0;
}

function resolveResponsesReasoning(
  model: Model<any>,
  options: WsOptions | undefined,
): { effort?: "none" | "minimal" | "low" | "medium" | "high" | "xhigh"; summary?: "auto" | "concise" | "detailed" | null } | undefined {
  if (!model.reasoning) return undefined;
  const configured = thinkingLevelToResponsesReasoning(options?.reasoning);
  if (!configured) {
    return model.provider !== "github-copilot" ? { effort: "none" } : undefined;
  }
  return {
    effort: configured.effort,
    summary: options?.reasoningSummary ?? configured.summary ?? "auto",
  };
}

class LocalAssistantMessageEventStream implements AssistantMessageEventStreamLike {
  private readonly queue: AssistantMessageEvent[] = [];
  private readonly waiting: Array<(value: IteratorResult<AssistantMessageEvent>) => void> = [];
  private done = false;
  private readonly finalResultPromise: Promise<AssistantMessage>;
  private resolveFinalResult!: (result: AssistantMessage) => void;

  constructor() {
    this.finalResultPromise = new Promise((resolve) => {
      this.resolveFinalResult = resolve;
    });
  }

  push(event: AssistantMessageEvent): void {
    if (this.done) return;
    if (event.type === "done") {
      this.done = true;
      this.resolveFinalResult(event.message);
    } else if (event.type === "error") {
      this.done = true;
      this.resolveFinalResult(event.error);
    }
    const waiter = this.waiting.shift();
    if (waiter) {
      waiter({ value: event, done: false });
      return;
    }
    this.queue.push(event);
  }

  end(result?: AssistantMessage): void {
    this.done = true;
    if (result) this.resolveFinalResult(result);
    while (this.waiting.length > 0) {
      const waiter = this.waiting.shift();
      waiter?.({ value: undefined as unknown as AssistantMessageEvent, done: true });
    }
  }

  async *[Symbol.asyncIterator](): AsyncIterator<AssistantMessageEvent> {
    while (true) {
      if (this.queue.length > 0) {
        yield this.queue.shift()!;
        continue;
      }
      if (this.done) return;
      const result = await new Promise<IteratorResult<AssistantMessageEvent>>((resolve) => {
        this.waiting.push(resolve);
      });
      if (result.done) return;
      yield result.value;
    }
  }

  result(): Promise<AssistantMessage> {
    return this.finalResultPromise;
  }
}

function createEventStream(): AssistantMessageEventStream {
  return typeof createAssistantMessageEventStream === "function"
    ? createAssistantMessageEventStream()
    : (new LocalAssistantMessageEventStream() as unknown as AssistantMessageEventStream);
}

function asResponsesModel(model: Model<any>): ResponsesModel {
  return model as ResponsesModel;
}

function getModelDescriptor(model: { api: string; provider: string; id: string }): ModelDescriptor {
  return { api: model.api, provider: model.provider, id: model.id };
}

const releaseWsSessionEffect = Effect.fn("OpenAIWebSocket.releaseSession")(function* (
  sessionId: string | undefined,
) {
  if (!sessionId) return;
  const registry = yield* WsSessionRegistry;
  const session = registry.sessions.get(sessionId);
  if (!session) return;
  registry.sessions.delete(sessionId);
  yield* session.manager.closeEffect().pipe(Effect.catchCause(() => Effect.void));
});

const releaseAllWsSessionsEffect = Effect.fn("OpenAIWebSocket.releaseAllSessions")(function* () {
  const registry = yield* WsSessionRegistry;
  for (const sessionId of [...registry.sessions.keys()]) yield* releaseWsSessionEffect(sessionId);
  registry.locks.clear();
});

/** Synchronous extension-lifecycle adapter. */
export function releaseWsSession(sessionId: string | undefined): void {
  openAIWebSocketRuntime.runSync(releaseWsSessionEffect(sessionId));
}

/** Synchronous extension-lifecycle adapter. */
export function releaseAllWsSessions(): void {
  for (const fiber of activeStreamFibers) fiber.interruptUnsafe();
  activeStreamFibers.clear();
  openAIWebSocketRuntime.runSync(releaseAllWsSessionsEffect());
}

function toNonEmptyString(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function normalizeAssistantPhase(value: unknown): OpenAIResponsesAssistantPhase | undefined {
  return value === "commentary" || value === "final_answer" ? value : undefined;
}

function responseFromEvent(event: OpenAIWebSocketEvent): ResponseObject | undefined {
  const response = (event as { response?: unknown }).response;
  return isResponseObject(response) ? response : undefined;
}

function eventString(event: OpenAIWebSocketEvent, key: string): string | undefined {
  const value = (event as Record<string, unknown>)[key];
  return typeof value === "string" ? value : undefined;
}

function encodeAssistantTextSignature(params: {
  id: string;
  phase?: OpenAIResponsesAssistantPhase;
}): string {
  return JSON.stringify({
    v: 1,
    id: params.id,
    ...(params.phase ? { phase: params.phase } : {}),
  });
}

function parseAssistantTextSignature(
  value: unknown,
): { id: string; phase?: OpenAIResponsesAssistantPhase } | null {
  if (typeof value !== "string" || value.trim().length === 0) return null;
  if (!value.startsWith("{")) return { id: value };
  try {
    const parsed = JSON.parse(value) as { v?: unknown; id?: unknown; phase?: unknown };
    if (parsed.v !== 1 || typeof parsed.id !== "string") return null;
    return {
      id: parsed.id,
      ...(normalizeAssistantPhase(parsed.phase) ? { phase: normalizeAssistantPhase(parsed.phase) } : {}),
    };
  } catch {
    return null;
  }
}

function supportsImageInput(modelOverride?: ReplayModelInfo): boolean {
  return !Array.isArray(modelOverride?.input) || modelOverride.input.includes("image");
}

function contentToText(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .filter((part): part is { type?: string; text?: string } => Boolean(part) && typeof part === "object")
    .filter(
      (part) =>
        (part.type === "text" || part.type === "input_text" || part.type === "output_text") &&
        typeof part.text === "string",
    )
    .map((part) => part.text as string)
    .join("");
}

function contentToOpenAIParts(content: unknown, modelOverride?: ReplayModelInfo): ContentPart[] {
  if (typeof content === "string") {
    return content ? [{ type: "input_text", text: content }] : [];
  }
  if (!Array.isArray(content)) return [];

  const includeImages = supportsImageInput(modelOverride);
  const parts: ContentPart[] = [];
  for (const part of content as Array<{
    type?: string;
    text?: string;
    data?: string;
    mimeType?: string;
    source?: unknown;
  }>) {
    if (
      (part.type === "text" || part.type === "input_text" || part.type === "output_text") &&
      typeof part.text === "string"
    ) {
      parts.push({ type: "input_text", text: part.text });
      continue;
    }
    if (!includeImages) continue;
    if (part.type === "image" && typeof part.data === "string") {
      parts.push({
        type: "input_image",
        source: {
          type: "base64",
          media_type: part.mimeType ?? "image/jpeg",
          data: part.data,
        },
      });
      continue;
    }
    if (
      part.type === "input_image" &&
      part.source &&
      typeof part.source === "object" &&
      typeof (part.source as { type?: unknown }).type === "string"
    ) {
      parts.push({
        type: "input_image",
        source: part.source as
          | { type: "url"; url: string }
          | { type: "base64"; media_type: string; data: string },
      });
    }
  }
  return parts;
}

function parseReasoningItem(value: unknown): Extract<InputItem, { type: "reasoning" }> | null {
  if (!value || typeof value !== "object") return null;
  const record = value as {
    type?: unknown;
    content?: unknown;
    encrypted_content?: unknown;
    summary?: unknown;
  };
  if (record.type !== "reasoning") return null;
  return {
    type: "reasoning",
    ...(typeof record.content === "string" ? { content: record.content } : {}),
    ...(typeof record.encrypted_content === "string" ? { encrypted_content: record.encrypted_content } : {}),
    ...(typeof record.summary === "string" ? { summary: record.summary } : {}),
  };
}

function parseThinkingSignature(value: unknown): Extract<InputItem, { type: "reasoning" }> | null {
  if (typeof value !== "string" || value.trim().length === 0) return null;
  try {
    return parseReasoningItem(JSON.parse(value));
  } catch {
    return null;
  }
}

function convertTools(tools: Context["tools"]): FunctionToolDefinition[] {
  if (!tools || tools.length === 0) return [];
  return tools.map((tool) => ({
    type: "function",
    name: tool.name,
    description: typeof tool.description === "string" ? tool.description : undefined,
    parameters: (tool.parameters ?? {}) as Record<string, unknown>,
  }));
}

function convertMessagesToInputItems(messages: Message[], modelOverride?: ReplayModelInfo): InputItem[] {
  const items: InputItem[] = [];

  for (const msg of messages) {
    const m = msg as AnyMessage;

    if (m.role === "user") {
      const parts = contentToOpenAIParts(m.content, modelOverride);
      if (parts.length === 0) continue;
      items.push({
        type: "message",
        role: "user",
        content:
          parts.length === 1 && parts[0]?.type === "input_text"
            ? (parts[0] as { type: "input_text"; text: string }).text
            : parts,
      });
      continue;
    }

    if (m.role === "assistant") {
      let assistantPhase = normalizeAssistantPhase(m.phase);
      if (Array.isArray(m.content)) {
        const textParts: string[] = [];
        const pushAssistantText = () => {
          if (textParts.length === 0) return;
          items.push({
            type: "message",
            role: "assistant",
            content: textParts.join(""),
            ...(assistantPhase ? { phase: assistantPhase } : {}),
          });
          textParts.length = 0;
        };

        for (const block of m.content as Array<{
          type?: string;
          text?: string;
          textSignature?: unknown;
          id?: unknown;
          name?: unknown;
          arguments?: unknown;
          thinkingSignature?: unknown;
        }>) {
          if (block.type === "text" && typeof block.text === "string") {
            const parsedSignature = parseAssistantTextSignature(block.textSignature);
            if (!assistantPhase) assistantPhase = parsedSignature?.phase;
            textParts.push(block.text);
            continue;
          }
          if (block.type === "thinking") {
            pushAssistantText();
            const reasoningItem = parseThinkingSignature(block.thinkingSignature);
            if (reasoningItem) items.push(reasoningItem);
            continue;
          }
          if (block.type !== "toolCall") continue;

          pushAssistantText();
          const callIdRaw = toNonEmptyString(block.id);
          const toolName = toNonEmptyString(block.name);
          if (!callIdRaw || !toolName) continue;
          const [callId, itemId] = callIdRaw.split("|", 2);
          items.push({
            type: "function_call",
            ...(itemId ? { id: itemId } : {}),
            call_id: callId,
            name: toolName,
            arguments:
              typeof block.arguments === "string"
                ? block.arguments
                : JSON.stringify(block.arguments ?? {}),
          });
        }

        pushAssistantText();
        continue;
      }

      const text = contentToText(m.content);
      if (!text) continue;
      items.push({
        type: "message",
        role: "assistant",
        content: text,
        ...(assistantPhase ? { phase: assistantPhase } : {}),
      });
      continue;
    }

    if (m.role !== "toolResult") continue;

    const toolCallId = toNonEmptyString(m.toolCallId) ?? toNonEmptyString(m.toolUseId);
    if (!toolCallId) continue;
    const [callId] = toolCallId.split("|", 2);
    const parts = Array.isArray(m.content) ? contentToOpenAIParts(m.content, modelOverride) : [];
    const textOutput = contentToText(m.content);
    const imageParts = parts.filter((part) => part.type === "input_image");
    items.push({
      type: "function_call_output",
      call_id: callId,
      output: textOutput || (imageParts.length > 0 ? "(see attached image)" : ""),
    });
    if (imageParts.length > 0) {
      items.push({
        type: "message",
        role: "user",
        content: [{ type: "input_text", text: "Attached image(s) from tool result:" }, ...imageParts],
      });
    }
  }

  return items;
}

function buildAssistantMessageFromResponse(
  response: ResponseObject,
  model: Model<any>,
  serviceTier?: "auto" | "default" | "flex" | "priority",
): AssistantMessage {
  const modelInfo = getModelDescriptor(model);
  const content: (TextContent | ToolCall)[] = [];
  let assistantPhase: OpenAIResponsesAssistantPhase | undefined;

  for (const item of response.output ?? []) {
    if (item.type === "message") {
      const itemPhase = normalizeAssistantPhase(item.phase);
      if (itemPhase) assistantPhase = itemPhase;
      for (const part of item.content ?? []) {
        if (part.type === "output_text" && part.text) {
          content.push({
            type: "text",
            text: part.text,
            textSignature: encodeAssistantTextSignature({
              id: item.id,
              ...(itemPhase ? { phase: itemPhase } : {}),
            }),
          });
        }
      }
    } else if (item.type === "function_call") {
      const toolName = toNonEmptyString(item.name);
      if (!toolName) continue;
      content.push({
        type: "toolCall",
        id: toNonEmptyString(item.call_id) ?? `call_${randomUUID()}`,
        name: toolName,
        arguments: (() => {
          try {
            return JSON.parse(item.arguments) as Record<string, unknown>;
          } catch {
            return {};
          }
        })(),
      });
    }
  }

  const hasToolCalls = content.some((c) => c.type === "toolCall");
  const stopReason: StopReason = hasToolCalls ? "toolUse" : "stop";
  const cachedTokens = response.usage?.input_tokens_details?.cached_tokens ?? 0;
  const cacheWriteTokens = extractCacheWriteTokens(response);
  const usage: Usage = {
    input: Math.max(0, (response.usage?.input_tokens ?? 0) - cachedTokens - cacheWriteTokens),
    output: response.usage?.output_tokens ?? 0,
    cacheRead: cachedTokens,
    cacheWrite: cacheWriteTokens,
    totalTokens: response.usage?.total_tokens ?? 0,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
  };
  calculateCost(model, usage);
  applyServiceTierPricing(usage, modelInfo, response.service_tier ?? serviceTier);
  const message = buildAssistantMessage({
    model: modelInfo,
    content,
    stopReason,
    usage,
    responseId: response.id,
  });

  return assistantPhase
    ? ({ ...message, phase: assistantPhase } as AssistantMessageWithPhase)
    : message;
}

function resolveWsTransport(options: SimpleStreamOptions | undefined): WsTransport {
  const transport = (options as { transport?: unknown } | undefined)?.transport;
  return transport === "sse" || transport === "websocket" || transport === "auto"
    ? transport
    : "auto";
}

function resolveWsWarmup(options: SimpleStreamOptions | undefined): boolean {
  const warmup = (options as WsOptions | undefined)?.openaiWsWarmup;
  return warmup === true;
}

function buildWsRequestKey(params: {
  model: Model<any>;
  context: Context;
  tools: FunctionToolDefinition[];
  options: WsOptions | undefined;
}): string {
  return JSON.stringify({
    model: params.model.id,
    instructions: params.context.systemPrompt ?? undefined,
    tools: params.tools.length > 0 ? params.tools : undefined,
    temperature: params.options?.temperature,
    max_output_tokens: params.options?.maxTokens,
    top_p: params.options?.topP,
    tool_choice: params.options?.toolChoice,
    service_tier: params.options?.serviceTier,
    reasoning: resolveResponsesReasoning(params.model, params.options),
    text: params.options?.text,
    metadata: params.options?.metadata,
  });
}

export function selectInputItemsForContinuation(params: {
  context: Context;
  model: ReplayModelInfo;
  session: Pick<WsSession, "lastContextLength">;
  currentModelKey: string;
  remoteCompactionState: ReturnType<typeof getRemoteCompactionState>;
  previousResponseId?: string | null;
}): Array<InputItem | Record<string, unknown>> {
  const { context, model, session, currentModelKey, remoteCompactionState, previousResponseId } = params;

  if (remoteCompactionState && remoteCompactionState.modelKey === currentModelKey) {
    return normalizeResponseItemsForPrompt(
      remoteCompactionState.explicitHistory,
      model,
    ) as Array<InputItem | Record<string, unknown>>;
  }

  if (previousResponseId && session.lastContextLength > 0) {
    const newMessages = context.messages.slice(session.lastContextLength);
    if (newMessages.length > 0) {
      return convertMessagesToInputItems(newMessages, model);
    }
  }

  return buildFullInput(context, model);
}

function buildResponseCreatePayload(params: {
  model: Model<any>;
  context: Context;
  inputItems: Array<InputItem | Record<string, unknown>>;
  tools: FunctionToolDefinition[];
  previousResponseId?: string | null;
  options: WsOptions | undefined;
}): Record<string, unknown> {
  const reasoning = resolveResponsesReasoning(params.model, params.options);
  return {
    type: "response.create",
    model: params.model.id,
    store: false,
    input: params.inputItems,
    instructions: params.context.systemPrompt ?? undefined,
    tools: params.tools.length > 0 ? params.tools : undefined,
    ...(params.previousResponseId ? { previous_response_id: params.previousResponseId } : {}),
    ...(params.options?.temperature !== undefined ? { temperature: params.options.temperature } : {}),
    ...(params.options?.maxTokens !== undefined ? { max_output_tokens: params.options.maxTokens } : {}),
    ...(params.options?.topP !== undefined ? { top_p: params.options.topP } : {}),
    ...(params.options?.toolChoice !== undefined ? { tool_choice: params.options.toolChoice } : {}),
    ...(params.options?.serviceTier !== undefined ? { service_tier: params.options.serviceTier } : {}),
    ...(reasoning ? { reasoning } : {}),
    ...(reasoning && reasoning.effort && reasoning.effort !== "none"
      ? { include: ["reasoning.encrypted_content"] }
      : {}),
    ...(params.options?.text ? { text: params.options.text } : {}),
    ...(params.options?.metadata ? { metadata: params.options.metadata as Record<string, string> } : {}),
  };
}

export class OpenAIWebSocketStreamError extends Schema.TaggedError<OpenAIWebSocketStreamError>()(
  "OpenAIWebSocketStreamError",
  { message: Schema.String, cause: Schema.optional(Schema.Defect()) },
) {}

const streamError = (cause: unknown, prefix?: string) => new OpenAIWebSocketStreamError({
  message: prefix
    ? `${prefix}: ${cause instanceof Error ? cause.message : String(cause)}`
    : cause instanceof Error ? cause.message : String(cause),
  cause,
});

type ManagerSignal =
  | { readonly type: "message"; readonly event: OpenAIWebSocketEvent }
  | { readonly type: "close"; readonly code: number; readonly reason: string }
  | { readonly type: "abort" };

const acquireManagerSignals = Effect.fn("OpenAIWebSocket.acquireManagerSignals")(function* (
  manager: OpenAIWebSocketManager,
  signal?: AbortSignal,
) {
  const queue = yield* Queue.unbounded<ManagerSignal>();
  let active = true;
  const offer = (value: ManagerSignal) => {
    if (!Queue.offerUnsafe(queue, value) && active) {
      throw new Error("OpenAI WebSocket unbounded event queue rejected an event");
    }
  };
  const messageHandler = (event: OpenAIWebSocketEvent) => offer({ type: "message", event });
  const closeHandler = (code: number, reason: string) => offer({ type: "close", code, reason });
  const abortHandler = () => offer({ type: "abort" });
  yield* Effect.acquireRelease(
    Effect.sync(() => {
      manager.on("message", messageHandler);
      manager.on("close", closeHandler);
      signal?.addEventListener("abort", abortHandler, { once: true });
      if (signal?.aborted) abortHandler();
    }),
    () => Effect.sync(() => {
      active = false;
      manager.off("message", messageHandler);
      manager.off("close", closeHandler);
      signal?.removeEventListener("abort", abortHandler);
    }).pipe(Effect.andThen(Queue.shutdown(queue))),
  );
  return queue;
});

const runWarmUpBase = Effect.fn("OpenAIWebSocket.warmUp")(function* (params: {
  manager: OpenAIWebSocketManager;
  modelId: string;
  tools: FunctionToolDefinition[];
  instructions?: string;
  signal?: AbortSignal;
}) {
  const signals = yield* acquireManagerSignals(params.manager, params.signal);
  yield* params.manager.warmUpEffect({
    model: params.modelId,
    tools: params.tools.length > 0 ? params.tools : undefined,
    instructions: params.instructions,
  });
  while (true) {
    const signal = yield* Queue.take(signals);
    if (signal.type === "abort") return yield* Effect.fail(streamError("aborted"));
    if (signal.type === "close") {
      return yield* Effect.fail(streamError(
        `warm-up closed (code=${signal.code}, reason=${signal.reason || "unknown"})`,
      ));
    }
    const event = signal.event;
    if (event.type === "response.completed") return;
    if (event.type === "response.failed") {
      return yield* Effect.fail(streamError(
        `warm-up failed: ${responseFromEvent(event)?.error?.message ?? "Response failed"}`,
      ));
    }
    if (event.type === "error") {
      return yield* Effect.fail(streamError(
        `warm-up error: ${eventString(event, "message") ?? "Unknown error"} ` +
          `(code=${eventString(event, "code") ?? "unknown"})`,
      ));
    }
  }
});

function runWarmUp(params: Parameters<typeof runWarmUpBase>[0]) {
  return runWarmUpBase(params).pipe(
    Effect.scoped,
    Effect.timeout(WARM_UP_TIMEOUT_MS),
    Effect.mapError((cause) => streamError(cause, `warm-up timed out after ${WARM_UP_TIMEOUT_MS}ms`)),
  );
}

function buildFullInput(context: Context, model: ReplayModelInfo): InputItem[] {
  return convertMessagesToInputItems(context.messages, model);
}

const fallbackToHttp = Effect.fn("OpenAIWebSocket.fallbackToHttp")(function* (
  model: ResponsesModel,
  context: Context,
  options: SimpleStreamOptions | undefined,
  eventStream: AssistantMessageEventStreamLike,
  signal?: AbortSignal,
) {
  const sessionId = options?.sessionId;
  const remoteCompactionState = sessionId ? getRemoteCompactionState(sessionId) : undefined;
  const continuationState = sessionId ? getContinuationState(sessionId) : undefined;
  const originalOnPayload = options?.onPayload;
  const mergedOptions = {
    ...(signal ? { ...options, signal } : options),
    onPayload: async (payload: unknown, payloadModel: Model<any>) => {
      let nextPayload = payload;
      if (payload && typeof payload === "object") {
        const payloadObj = { ...(payload as Record<string, unknown>) };
        if (remoteCompactionState && remoteCompactionState.modelKey === modelKey(model)) {
          payloadObj.input = normalizeResponseItemsForPrompt(remoteCompactionState.explicitHistory, model) as unknown[];
          delete payloadObj.previous_response_id;
          nextPayload = payloadObj;
        } else if (
          typeof payloadObj.previous_response_id === "string" &&
          continuationState?.modelKey === modelKey(model) &&
          typeof continuationState.contextLength === "number" &&
          continuationState.contextLength > 0
        ) {
          const newMessages = context.messages.slice(continuationState.contextLength);
          if (newMessages.length > 0) {
            payloadObj.input = convertMessagesToInputItems(newMessages, model) as unknown[];
            nextPayload = payloadObj;
          }
        }
      }
      const chained = await originalOnPayload?.(nextPayload, payloadModel);
      return chained ?? nextPayload;
    },
  } satisfies SimpleStreamOptions | undefined;
  const httpStream = yield* Effect.try({
    try: () => streamSimpleOpenAIResponses(model, context, mergedOptions),
    catch: (cause) => streamError(cause, "OpenAI HTTP fallback could not start"),
  });
  yield* Stream.fromAsyncIterable(
    httpStream,
    (cause) => streamError(cause, "OpenAI HTTP fallback failed"),
  ).pipe(Stream.runForEach((event) => Effect.sync(() => eventStream.push(event))));
});

function fallbackToHttpResponses(
  model: Model<any>,
  context: Context,
  options: SimpleStreamOptions | undefined,
  eventStream: AssistantMessageEventStreamLike,
  signal?: AbortSignal,
) {
  return fallbackToHttp(asResponsesModel(model), context, options, eventStream, signal);
}

const runOpenAIWebSocketStream = Effect.fn("OpenAIWebSocket.runStream")(function* (params: {
  model: Model<any>;
  context: Context;
  options: SimpleStreamOptions | undefined;
  eventStream: AssistantMessageEventStreamLike;
  managerOptions: OpenAIWebSocketManagerOptions;
  httpFallback?: typeof fallbackToHttpResponses;
}) {
  const { model, context, options, eventStream, managerOptions } = params;
  const httpFallback = params.httpFallback ?? fallbackToHttpResponses;
  const cfg = loadConfig(process.cwd());
  const modelInfo = getModelDescriptor(model);
  if (!cfg.enabled || !cfg.usePreviousResponseId || !isDirectOpenAIResponsesModel(model)) {
    return yield* httpFallback(model, context, options, eventStream);
  }

  const sessionId = options?.sessionId;
  const apiKey = options?.apiKey;
  const transport = resolveWsTransport(options);
  if (transport === "sse") {
    return yield* httpFallback(model, context, options, eventStream);
  }
  if (!sessionId || !apiKey) {
    if (transport === "websocket") {
      return yield* Effect.fail(streamError(
        `WebSocket transport requires ${!sessionId ? "a sessionId" : "an apiKey"}.`,
      ));
    }
    return yield* httpFallback(model, context, options, eventStream);
  }

  const registry = yield* WsSessionRegistry;
  let lock = registry.locks.get(sessionId);
  if (!lock) {
    lock = Semaphore.makeUnsafe(1);
    registry.locks.set(sessionId, lock);
  }
  return yield* lock.withPermit(Effect.gen(function* () {
  let session = registry.sessions.get(sessionId);
  const currentModelKey = modelKey(model);
  if (session && session.modelKey !== currentModelKey) {
    yield* releaseWsSessionEffect(sessionId);
    session = undefined;
  }
  if (!session) {
    const headers = {
      ...(managerOptions.headers ?? {}),
      ...buildCodexWebSocketHeaders(sessionId),
    };
    session = {
      manager: new OpenAIWebSocketManager({ ...managerOptions, headers }),
      modelKey: currentModelKey,
      lastContextLength: 0,
      lastRequestKey: undefined,
      warmUpAttempted: false,
      broken: false,
    };
    registry.sessions.set(sessionId, session);
  }

  if (!session.manager.isConnected() && !session.broken) {
    const connected = yield* Effect.exit(session.manager.connectEffect(apiKey));
    if (Exit.isFailure(connected)) {
      session.broken = true;
      registry.sessions.delete(sessionId);
      yield* session.manager.closeEffect().pipe(Effect.catchCause(() => Effect.void));
      if (transport === "websocket") {
        return yield* Effect.fail(streamError(Cause.squash(connected.cause)));
      }
      return yield* httpFallback(model, context, options, eventStream);
    }
  }

  if (session.broken || !session.manager.isConnected()) {
    if (transport === "websocket") {
      return yield* Effect.fail(streamError("WebSocket session disconnected"));
    }
    yield* releaseWsSessionEffect(sessionId);
    return yield* httpFallback(model, context, options, eventStream);
  }

  const signal = options?.signal;
  if (resolveWsWarmup(options) && !session.warmUpAttempted) {
    session.warmUpAttempted = true;
    yield* runWarmUp({
      manager: session.manager,
      modelId: model.id,
      tools: convertTools(context.tools),
      instructions: context.systemPrompt ?? undefined,
      signal,
    }).pipe(Effect.catchCause(() => Effect.void));
  }

  const remoteCompactionState = getRemoteCompactionState(sessionId);
  const continuationState = getContinuationState(sessionId);
  const typedOptions = options as WsOptions | undefined;
  const functionTools = convertTools(context.tools);
  const requestKey = buildWsRequestKey({ model, context, tools: functionTools, options: typedOptions });
  const incrementalAllowed = session.lastRequestKey === undefined || session.lastRequestKey === requestKey;
  const prevResponseId =
    remoteCompactionState && remoteCompactionState.modelKey === currentModelKey
      ? undefined
      : incrementalAllowed
        ? session.manager.previousResponseId ??
          (continuationState?.modelKey === currentModelKey ? continuationState.responseId : undefined)
        : undefined;
  const baselineContextLength =
    continuationState?.modelKey === currentModelKey && typeof continuationState.contextLength === "number"
      ? continuationState.contextLength
      : session.lastContextLength;
  const inputItems = selectInputItemsForContinuation({
    context,
    model,
    session: { lastContextLength: baselineContextLength },
    currentModelKey,
    remoteCompactionState,
    previousResponseId: prevResponseId,
  });
  const payload = buildResponseCreatePayload({
    model,
    context,
    inputItems,
    tools: functionTools,
    previousResponseId: prevResponseId,
    options: typedOptions,
  });
  const nextPayload = options?.onPayload
    ? yield* Effect.tryPromise({
        try: () => Promise.resolve(options.onPayload!(payload, model)),
        catch: (cause) => streamError(cause, "OpenAI WebSocket payload callback failed"),
      }).pipe(Effect.map((next) => next ?? payload))
    : payload;

  let requestAttempted = false;
  const requestExit = yield* Effect.exit(Effect.gen(function* () {
    const signals = yield* acquireManagerSignals(session.manager, signal);
    if (signal?.aborted) return yield* Effect.fail(streamError("aborted"));
    requestAttempted = true;
    yield* session.manager.sendEffect(
      nextPayload as Parameters<OpenAIWebSocketManager["send"]>[0],
    ).pipe(Effect.mapError((cause) => streamError(cause)));
    session.lastRequestKey = requestKey;
    eventStream.push({
      type: "start",
      partial: buildAssistantMessageWithZeroUsage({ model: modelInfo, content: [], stopReason: "stop" }),
    });

    const capturedContextLength = context.messages.length;
    let textStarted = false;
    while (true) {
      const managerSignal = yield* Queue.take(signals);
      if (managerSignal.type === "abort") return yield* Effect.fail(streamError("aborted"));
      if (managerSignal.type === "close") {
        return yield* Effect.fail(streamError(
          `WebSocket closed mid-request (code=${managerSignal.code}, reason=${managerSignal.reason || "unknown"})`,
        ));
      }
      const event = managerSignal.event;
      if (event.type === "response.completed") {
        const response = responseFromEvent(event);
        if (!response) {
          return yield* Effect.fail(streamError("OpenAI WebSocket completed event had no valid response."));
        }
        session.lastContextLength = capturedContextLength;
        const assistantMsg = buildAssistantMessageFromResponse(response, model, typedOptions?.serviceTier);
        setContinuationState(sessionId, {
          responseId: response.id,
          modelKey: currentModelKey,
          updatedAt: Date.now(),
          contextLength: capturedContextLength,
        });
        const reason: Extract<StopReason, "stop" | "length" | "toolUse"> =
          assistantMsg.stopReason === "toolUse" ? "toolUse" : "stop";
        eventStream.push({ type: "done", reason, message: assistantMsg });
        return;
      }
      if (event.type === "response.failed") {
        return yield* Effect.fail(streamError(
          `OpenAI WebSocket response failed: ${responseFromEvent(event)?.error?.message ?? "Response failed"}`,
        ));
      }
      if (event.type === "error") {
        return yield* Effect.fail(streamError(
          `OpenAI WebSocket error: ${eventString(event, "message") ?? "Unknown error"} ` +
            `(code=${eventString(event, "code") ?? "unknown"})`,
        ));
      }
      if (event.type !== "response.output_text.delta") continue;
      const delta = eventString(event, "delta");
      if (delta === undefined) continue;
      const partialMsg = buildAssistantMessageWithZeroUsage({
        model: modelInfo,
        content: [{ type: "text", text: delta }],
        stopReason: "stop",
      });
      if (!textStarted) {
        textStarted = true;
        eventStream.push({ type: "text_start", contentIndex: 0, partial: partialMsg });
      }
      eventStream.push({ type: "text_delta", contentIndex: 0, delta, partial: partialMsg });
    }
  }).pipe(Effect.scoped));

  if (Exit.isSuccess(requestExit)) return;
  yield* releaseWsSessionEffect(sessionId);
  if (!requestAttempted && transport !== "websocket") {
    return yield* httpFallback(model, context, options, eventStream, signal);
  }
  return yield* Effect.fail(streamError(Cause.squash(requestExit.cause)));
  }));
});

export function createOpenAIWebSocketStreamFn(
  managerOptions: OpenAIWebSocketManagerOptions = {},
  dependencies: { httpFallback?: typeof fallbackToHttpResponses } = {},
): StreamFunction {
  return (model, context, options) => {
    const eventStream = createEventStream();
    const program = runOpenAIWebSocketStream({
      model,
      context,
      options,
      eventStream,
      managerOptions,
      httpFallback: dependencies.httpFallback,
    }).pipe(
      Effect.catchCause((cause) => releaseWsSessionEffect(options?.sessionId).pipe(
        Effect.andThen(Effect.sync(() => {
          const failure = Cause.squash(cause);
          eventStream.push({
            type: "error",
            reason: options?.signal?.aborted ? "aborted" : "error",
            error: buildStreamErrorAssistantMessage({
              model: getModelDescriptor(model),
              errorMessage: failure instanceof Error ? failure.message : String(failure),
            }),
          });
          eventStream.end();
        })),
      )),
    );

    // Pi's provider is synchronous; this is the single runtime launch adapter.
    const fiber = openAIWebSocketRuntime.runFork(program);
    activeStreamFibers.add(fiber);
    fiber.addObserver(() => activeStreamFibers.delete(fiber));
    return eventStream;
  };
}
