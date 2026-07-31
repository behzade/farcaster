/**
 * Runs Codex compaction through Pi AI's normal Responses stream.
 *
 * This matters for Codex WebSockets: the provider can turn the full input made
 * here into a previous_response_id request when it matches the active stream's
 * saved request and response. The raw HTTP fallback cannot use that in-process
 * continuation state.
 */
import type { ResponseItem, RemoteCompactionUsageSnapshot } from "./remote-compaction.ts";
import { isRecord } from "./config.ts";
import { Context, Data, Effect, Layer, Stream } from "effect";

export const RESPONSE_OUTPUT_ITEM_HOOK_VERSION = 1;

export type CompactionRequestShape = {
  instructions?: string;
  tools?: unknown[];
  parallelToolCalls?: boolean;
  toolChoice?: unknown;
  reasoning?: unknown;
  text?: unknown;
  serviceTier?: unknown;
};

type AssistantMessageLike = {
  stopReason?: string;
  responseId?: string;
  errorMessage?: string;
  usage?: RemoteCompactionUsageSnapshot;
};

type StreamEventLike = {
  type?: string;
  reason?: string;
  message?: AssistantMessageLike;
  error?: AssistantMessageLike;
};

export type ContinuationCompactionStream = (
  model: unknown,
  context: unknown,
  options: Record<string, unknown>,
) => AsyncIterable<StreamEventLike>;

export interface ResponsesCompactionStreamShape {
  readonly run: ContinuationCompactionStream;
}

export class ResponsesCompactionStream extends Context.Tag(
  "pi-openai-server-compaction/ResponsesCompactionStream",
)<ResponsesCompactionStream, ResponsesCompactionStreamShape>() {}

export const responsesCompactionStreamLayer = (
  run: ContinuationCompactionStream,
): Layer.Layer<ResponsesCompactionStream> =>
  Layer.succeed(ResponsesCompactionStream, { run });

export class ContinuationCompactionError extends Data.TaggedError(
  "ContinuationCompactionError",
)<{ readonly message: string; readonly cause?: unknown }> {}

export type ContinuationCompactionResult = {
  compactionItem: ResponseItem;
  promptInput: ResponseItem[];
  responseId: string;
  usage?: RemoteCompactionUsageSnapshot;
};

function cloneItems(items: readonly ResponseItem[]): ResponseItem[] {
  return structuredClone(items) as ResponseItem[];
}

function applyRequestShape(
  body: Record<string, unknown>,
  shape: CompactionRequestShape | undefined,
): Record<string, unknown> {
  if (!shape) return body;
  return {
    ...body,
    ...(shape.instructions !== undefined ? { instructions: shape.instructions } : {}),
    ...(shape.tools !== undefined ? { tools: structuredClone(shape.tools) } : {}),
    ...(shape.parallelToolCalls !== undefined
      ? { parallel_tool_calls: shape.parallelToolCalls }
      : {}),
    ...(shape.toolChoice !== undefined ? { tool_choice: structuredClone(shape.toolChoice) } : {}),
    ...(shape.reasoning !== undefined ? { reasoning: structuredClone(shape.reasoning) } : {}),
    ...(shape.text !== undefined ? { text: structuredClone(shape.text) } : {}),
    ...(shape.serviceTier !== undefined ? { service_tier: shape.serviceTier } : {}),
  };
}

function isCompactionItem(value: unknown): value is ResponseItem {
  return isRecord(value) && value.type === "compaction" && typeof value.encrypted_content === "string";
}

export const executeContinuationCompaction = (params: {
  model: unknown;
  context: unknown;
  streamOptions: Record<string, unknown>;
  explicitPromptInput?: readonly ResponseItem[];
  requestShape?: CompactionRequestShape;
}): Effect.Effect<
  ContinuationCompactionResult,
  ContinuationCompactionError,
  ResponsesCompactionStream
> =>
  Effect.gen(function* () {
    const streamService = yield* ResponsesCompactionStream;
    const compactionItems: ResponseItem[] = [];
    let promptInput: ResponseItem[] | undefined;
    let completed: AssistantMessageLike | undefined;
    let completedNormally = false;

    const events = yield* Effect.try({
      try: () => streamService.run(params.model, params.context, {
        ...params.streamOptions,
        onOutputItemDone: (item: unknown) => {
          if (isCompactionItem(item)) compactionItems.push(structuredClone(item));
        },
        onPayload: (payload: unknown) => {
          if (!isRecord(payload)) {
            throw new Error("Codex compaction stream produced an invalid request body.");
          }
          const generatedInput = Array.isArray(payload.input)
            ? payload.input.filter(isRecord) as ResponseItem[]
            : [];
          promptInput = params.explicitPromptInput
            ? cloneItems(params.explicitPromptInput)
            : cloneItems(generatedInput);
          const body = applyRequestShape({ ...payload }, params.requestShape);
          delete body.previous_response_id;
          return {
            ...body,
            input: [...cloneItems(promptInput), { type: "compaction_trigger" }],
          };
        },
      }),
      catch: (cause) => new ContinuationCompactionError({
        message: "Codex compaction stream could not start.",
        cause,
      }),
    });

    yield* Stream.fromAsyncIterable(
      events,
      (cause) => new ContinuationCompactionError({
        message: "Codex compaction stream failed.",
        cause,
      }),
    ).pipe(
      Stream.runForEach((event) => {
        if (event.type === "done") {
          completed = event.message;
          completedNormally = event.reason === "stop" && event.message?.stopReason === "stop";
          return Effect.void;
        }
        if (event.type === "error") {
          return Effect.fail(new ContinuationCompactionError({
            message: event.error?.errorMessage || "Codex compaction stream failed.",
          }));
        }
        return Effect.void;
      }),
    );

    if (!completedNormally || !completed?.responseId) {
      return yield* Effect.fail(new ContinuationCompactionError({
        message: "Codex compaction stream did not complete normally.",
      }));
    }
    if (!promptInput) {
      return yield* Effect.fail(new ContinuationCompactionError({
        message: "Codex compaction stream did not build a request body.",
      }));
    }
    if (compactionItems.length !== 1) {
      return yield* Effect.fail(new ContinuationCompactionError({
        message: `Codex compaction expected one compaction output item, got ${compactionItems.length}.`,
      }));
    }
    const compactionItem = compactionItems[0];
    if (!compactionItem) {
      return yield* Effect.fail(new ContinuationCompactionError({
        message: "Codex compaction output item was missing.",
      }));
    }

    return {
      compactionItem,
      promptInput,
      responseId: completed.responseId,
      ...(completed.usage ? { usage: completed.usage } : {}),
    };
  });
