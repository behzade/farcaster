/**
 * Thin OpenAI Responses WebSocket client.
 *
 * Socket acquisition, listener ownership, reconnect delay, and shutdown are
 * represented as Effect resources. The class/EventEmitter and Promise methods
 * are retained only as compatibility adapters for Pi and focused fakes.
 */
import { EventEmitter } from "node:events";
import { Cause, Deferred, Effect, Exit, Fiber, Queue, Schema, Scope } from "effect";

export interface ResponseObject {
  id: string;
  object: "response";
  created_at: number;
  status: "in_progress" | "completed" | "failed" | "cancelled" | "incomplete";
  model: string;
  output: OutputItem[];
  usage?: UsageInfo;
  service_tier?: "auto" | "default" | "flex" | "priority";
  error?: { code: string; message: string };
}

export interface UsageInfo {
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  input_tokens_details?: {
    cached_tokens?: number;
    cache_creation_tokens?: number;
    cache_write_tokens?: number;
  };
}

export type OpenAIResponsesAssistantPhase = "commentary" | "final_answer";

export type OutputItem =
  | {
      type: "message";
      id: string;
      role: "assistant";
      content: Array<{ type: "output_text"; text: string }>;
      phase?: OpenAIResponsesAssistantPhase;
      status?: "in_progress" | "completed";
    }
  | {
      type: "function_call";
      id: string;
      call_id: string;
      name: string;
      arguments: string;
      status?: "in_progress" | "completed";
    }
  | {
      type: "reasoning";
      id: string;
      content?: string;
      summary?: string;
    };

export function isResponseObject(value: unknown): value is ResponseObject {
  if (!value || typeof value !== "object") return false;
  const response = value as { id?: unknown; output?: unknown };
  return typeof response.id === "string" && Array.isArray(response.output);
}

export interface ResponseCompletedEvent {
  type: "response.completed";
  response: ResponseObject;
}

export interface ResponseFailedEvent {
  type: "response.failed";
  response: ResponseObject;
}

export interface OutputTextDeltaEvent {
  type: "response.output_text.delta";
  item_id: string;
  output_index: number;
  content_index: number;
  delta: string;
}

export interface ErrorEvent {
  type: "error";
  code: string;
  message: string;
  param?: string;
}

export type OpenAIWebSocketEvent =
  | ResponseCompletedEvent
  | ResponseFailedEvent
  | OutputTextDeltaEvent
  | ErrorEvent
  | { type: string; [key: string]: unknown };

export type ContentPart =
  | { type: "input_text"; text: string }
  | { type: "output_text"; text: string }
  | {
      type: "input_image";
      source: { type: "url"; url: string } | { type: "base64"; media_type: string; data: string };
    };

export type InputItem =
  | {
      type: "message";
      role: "system" | "developer" | "user" | "assistant";
      content: string | ContentPart[];
      phase?: OpenAIResponsesAssistantPhase;
    }
  | { type: "function_call"; id?: string; call_id?: string; name: string; arguments: string }
  | { type: "function_call_output"; call_id: string; output: string }
  | { type: "reasoning"; content?: string; encrypted_content?: string; summary?: string }
  | { type: "item_reference"; id: string };

export type ToolChoice =
  | "auto"
  | "none"
  | "required"
  | { type: "function"; function: { name: string } };

export interface FunctionToolDefinition {
  type: "function";
  name: string;
  description?: string;
  parameters?: Record<string, unknown>;
  strict?: boolean;
}

export interface ResponseCreateEvent {
  type: "response.create";
  model: string;
  store?: boolean;
  stream?: boolean;
  input?: string | InputItem[];
  instructions?: string;
  tools?: FunctionToolDefinition[];
  tool_choice?: ToolChoice;
  context_management?: unknown;
  previous_response_id?: string;
  max_output_tokens?: number;
  temperature?: number;
  top_p?: number;
  metadata?: Record<string, string>;
  reasoning?: {
    effort?: "none" | "minimal" | "low" | "medium" | "high" | "xhigh";
    summary?: "auto" | "concise" | "detailed" | null;
  };
  truncation?: "auto" | "disabled";
  [key: string]: unknown;
}

export interface WarmUpEvent extends ResponseCreateEvent {
  generate: false;
}

export type ClientEvent = ResponseCreateEvent | WarmUpEvent;

const OPENAI_WS_URL = "wss://api.openai.com/v1/responses";
const MAX_RETRIES = 5;
const BACKOFF_DELAYS_MS = [1000, 2000, 4000, 8000, 16000] as const;
const WS_OPEN = 1;
const WS_CONNECTING = 0;

type WebSocketLike = EventEmitter & {
  readyState: number;
  send(data: string): void;
  close(code?: number, reason?: string): void;
  terminate?: () => void;
};

type SocketFactory = (url: string, options: { headers: Record<string, string> }) => WebSocketLike | Promise<WebSocketLike>;

export interface OpenAIWebSocketManagerOptions {
  url?: string;
  maxRetries?: number;
  backoffDelaysMs?: readonly number[];
  headers?: Record<string, string>;
  /** Test/adapter seam; production dynamically imports `ws`. */
  socketFactory?: SocketFactory;
}

export class OpenAIWebSocketTransportError extends Schema.TaggedError<OpenAIWebSocketTransportError>()(
  "OpenAIWebSocketTransportError",
  {
    message: Schema.String,
    cause: Schema.optional(Schema.Defect()),
  },
) {}

const transportError = (cause: unknown, prefix?: string) => new OpenAIWebSocketTransportError({
  message: prefix
    ? `${prefix}: ${cause instanceof Error ? cause.message : String(cause)}`
    : cause instanceof Error ? cause.message : String(cause),
  cause,
});

type SocketSignal =
  | { readonly type: "open" }
  | { readonly type: "error"; readonly error: Error }
  | { readonly type: "close"; readonly code: number; readonly reason: string }
  | { readonly type: "message"; readonly data: unknown };

export class OpenAIWebSocketManager extends EventEmitter {
  private ws: WebSocketLike | null = null;
  private apiKey: string | null = null;
  private retryCount = 0;
  private closed = false;
  private _previousResponseId: string | null = null;
  private readonly wsUrl: string;
  private readonly maxRetries: number;
  private readonly backoffDelaysMs: readonly number[];
  private readonly headers: Record<string, string>;
  private readonly socketFactory?: SocketFactory;
  private scope: Scope.Closeable = Scope.makeUnsafe();
  private supervisor: Fiber.Fiber<void, never> | undefined;
  private initialConnection: Deferred.Deferred<void, OpenAIWebSocketTransportError> | undefined;

  constructor(options: OpenAIWebSocketManagerOptions = {}) {
    super();
    this.wsUrl = options.url ?? OPENAI_WS_URL;
    this.maxRetries = options.maxRetries ?? MAX_RETRIES;
    this.backoffDelaysMs = options.backoffDelaysMs ?? BACKOFF_DELAYS_MS;
    this.headers = options.headers ?? {};
    this.socketFactory = options.socketFactory;
  }

  get previousResponseId(): string | null {
    return this._previousResponseId;
  }

  readonly connectEffect = Effect.fn("OpenAIWebSocketManager.connect")(function* (
    this: OpenAIWebSocketManager,
    apiKey: string,
  ) {
    if (this.supervisor) yield* this.closeEffect();
    if (this.closed) this.scope = Scope.makeUnsafe();
    this.apiKey = apiKey;
    this.closed = false;
    this.retryCount = 0;
    const initial = yield* Deferred.make<void, OpenAIWebSocketTransportError>();
    this.initialConnection = initial;
    this.supervisor = yield* Effect.forkIn(this.supervise(initial), this.scope);
    return yield* Deferred.await(initial).pipe(Effect.ensuring(Effect.sync(() => {
      if (this.initialConnection === initial) this.initialConnection = undefined;
    })));
  });

  /** Promise boundary retained for existing WebSocket-manager callers. */
  connect(apiKey: string): Promise<void> {
    return Effect.runPromise(this.connectEffect(apiKey));
  }

  readonly sendEffect = Effect.fn("OpenAIWebSocketManager.send")((event: ClientEvent) =>
    Effect.try({
      try: () => {
        if (!this.ws || this.ws.readyState !== WS_OPEN) {
          throw new Error(
            `OpenAIWebSocketManager: cannot send; connection not open (readyState=${this.ws?.readyState ?? "none"})`,
          );
        }
        this.ws.send(JSON.stringify(event));
      },
      catch: (cause) => transportError(cause),
    }),
  );

  send(event: ClientEvent): void {
    Effect.runSync(this.sendEffect(event));
  }

  onMessage(handler: (event: OpenAIWebSocketEvent) => void): () => void {
    this.on("message", handler);
    return () => this.off("message", handler);
  }

  isConnected(): boolean {
    return this.ws !== null && this.ws.readyState === WS_OPEN;
  }

  readonly closeEffect = Effect.fn("OpenAIWebSocketManager.close")(function* (this: OpenAIWebSocketManager) {
    if (this.closed) return;
    this.closed = true;
    const initial = this.initialConnection;
    this.initialConnection = undefined;
    if (initial && !Deferred.isDoneUnsafe(initial)) {
      Deferred.doneUnsafe(initial, Effect.fail(new OpenAIWebSocketTransportError({
        message: "OpenAIWebSocketManager: connection closed",
      })));
    }
    const socket = this.ws;
    this.ws = null;
    if (socket) {
      yield* Effect.sync(() => {
        try {
          if (socket.readyState === WS_OPEN) socket.close(1000, "Client closed");
          else if (socket.readyState === WS_CONNECTING) socket.terminate?.();
        } catch {
          // Shutdown is best effort; scope finalizers still remove owned listeners.
        }
      });
    }
    yield* Scope.close(this.scope, Exit.void);
    this.supervisor = undefined;
  });

  close(): void {
    Effect.runSync(this.closeEffect());
  }

  readonly warmUpEffect = Effect.fn("OpenAIWebSocketManager.warmUp")((params: {
    model: string;
    tools?: FunctionToolDefinition[];
    instructions?: string;
  }) => this.sendEffect({
    type: "response.create",
    generate: false,
    model: params.model,
    ...(params.tools ? { tools: params.tools } : {}),
    ...(params.instructions ? { instructions: params.instructions } : {}),
  }));

  warmUp(params: { model: string; tools?: FunctionToolDefinition[]; instructions?: string }): void {
    Effect.runSync(this.warmUpEffect(params));
  }

  private readonly createSocket = Effect.fn("OpenAIWebSocketManager.createSocket")(function* (this: OpenAIWebSocketManager) {
    if (!this.apiKey) return yield* Effect.fail(transportError("OpenAIWebSocketManager: apiKey is required."));
    const headers = {
      Authorization: `Bearer ${this.apiKey}`,
      "OpenAI-Beta": "responses_websockets=2026-02-06",
      ...this.headers,
    };
    if (this.socketFactory) {
      const socket = yield* Effect.try({
        try: () => this.socketFactory!(this.wsUrl, { headers }),
        catch: (cause) => transportError(cause, "OpenAIWebSocketManager: socket creation failed"),
      });
      if ("then" in socket && typeof socket.then === "function") {
        return yield* Effect.tryPromise({
          try: (signal) => socket.then((created) => {
            if (!signal.aborted) return created;
            created.terminate?.();
            throw new Error("OpenAIWebSocketManager: socket creation aborted");
          }),
          catch: (cause) => transportError(cause, "OpenAIWebSocketManager: socket creation failed"),
        });
      }
      return socket;
    }
    return yield* Effect.tryPromise({
      try: async (signal) => {
        const wsModule = await import("ws");
        if (signal.aborted) throw new Error("OpenAIWebSocketManager: socket creation aborted");
        const WebSocketCtor = (wsModule.default ?? wsModule) as unknown as new (
          url: string,
          options: { headers: Record<string, string> },
        ) => WebSocketLike;
        return new WebSocketCtor(this.wsUrl, { headers });
      },
      catch: (cause) => transportError(cause, "OpenAIWebSocketManager: socket creation failed"),
    });
  });

  private readonly supervise = Effect.fn("OpenAIWebSocketManager.supervise")(function* (
    this: OpenAIWebSocketManager,
    initial: Deferred.Deferred<void, OpenAIWebSocketTransportError>,
  ) {
    let firstAttempt = true;
    while (!this.closed) {
      const exit = yield* Effect.exit(this.runSocket(firstAttempt ? initial : undefined));
      if (firstAttempt) {
        firstAttempt = false;
        if (Exit.isFailure(exit) && !Deferred.isDoneUnsafe(initial)) {
          Deferred.doneUnsafe(initial, Effect.fail(transportError(
            Cause.squash(exit.cause),
            "OpenAIWebSocketManager: connection failed",
          )));
        }
      }
      if (this.closed) return;
      if (this.retryCount >= this.maxRetries) {
        this.emitError(new OpenAIWebSocketTransportError({
          message: `OpenAIWebSocketManager: max reconnect retries (${this.maxRetries}) exceeded.`,
        }));
        return;
      }
      const delayMs =
        this.backoffDelaysMs[Math.min(this.retryCount, this.backoffDelaysMs.length - 1)] ?? 1000;
      this.retryCount++;
      yield* Effect.sleep(delayMs);
    }
  });

  private readonly runSocketUnscoped = Effect.fn("OpenAIWebSocketManager.runSocket")(function* (
    this: OpenAIWebSocketManager,
    initial?: Deferred.Deferred<void, OpenAIWebSocketTransportError>,
  ) {
    const socket = yield* this.createSocket();
    const signals = yield* Queue.unbounded<SocketSignal>();
    let opened = false;
    const offer = (signal: SocketSignal) => {
      if (!Queue.offerUnsafe(signals, signal) && !this.closed) {
        this.emitError(new OpenAIWebSocketTransportError({
          message: "OpenAIWebSocketManager: internal unbounded socket queue was unexpectedly unavailable.",
        }));
      }
    };
    const onOpen = () => offer({ type: "open" });
    const onError = (error: Error) => offer({ type: "error", error });
    const onClose = (code: number, reason: Buffer | string) => offer({
      type: "close",
      code,
      reason: typeof reason === "string" ? reason : reason.toString(),
    });
    const onMessage = (data: unknown) => offer({ type: "message", data });

    yield* Effect.acquireRelease(
      Effect.sync(() => {
        this.ws = socket;
        socket.once("open", onOpen);
        socket.on("error", onError);
        socket.once("close", onClose);
        socket.on("message", onMessage);
      }),
      () => Effect.sync(() => {
        socket.off("open", onOpen);
        socket.off("error", onError);
        socket.off("close", onClose);
        socket.off("message", onMessage);
        if (this.ws === socket) this.ws = null;
        if (socket.readyState === WS_CONNECTING) socket.terminate?.();
      }).pipe(Effect.andThen(Queue.shutdown(signals))),
    );

    while (!this.closed) {
      const signal = yield* Queue.take(signals);
      if (signal.type === "open") {
        opened = true;
        this.retryCount = 0;
        if (initial && !Deferred.isDoneUnsafe(initial)) Deferred.doneUnsafe(initial, Effect.void);
        this.emit("open");
        continue;
      }
      if (signal.type === "message") {
        this.handleMessage(signal.data);
        continue;
      }
      if (signal.type === "error") {
        const error = transportError(signal.error);
        this.emitError(error);
        if (!opened) {
          if (initial && !Deferred.isDoneUnsafe(initial)) Deferred.doneUnsafe(initial, Effect.fail(error));
          return yield* Effect.fail(error);
        }
        continue;
      }

      if (this.ws === socket) this.ws = null;
      this.emit("close", signal.code, signal.reason);
      if (!opened) {
        const error = new OpenAIWebSocketTransportError({
          message: `OpenAIWebSocketManager: connection closed before open (code=${signal.code}, reason=${signal.reason || "unknown"})`,
        });
        if (initial && !Deferred.isDoneUnsafe(initial)) Deferred.doneUnsafe(initial, Effect.fail(error));
        return yield* Effect.fail(error);
      }
      return;
    }
  });

  private runSocket(initial?: Deferred.Deferred<void, OpenAIWebSocketTransportError>) {
    return this.runSocketUnscoped(initial).pipe(Effect.scoped);
  }

  private emitError(error: Error): void {
    if (this.listenerCount("error") > 0) this.emit("error", error);
  }

  private handleMessage(data: unknown): void {
    let text: string;
    if (typeof data === "string") text = data;
    else if (Buffer.isBuffer(data)) text = data.toString("utf8");
    else if (data instanceof ArrayBuffer) text = Buffer.from(data).toString("utf8");
    else text = String(data);

    let parsed: unknown;
    try {
      parsed = JSON.parse(text);
    } catch (cause) {
      this.emitError(transportError(cause, `OpenAIWebSocketManager: failed to parse message: ${text.slice(0, 200)}`));
      return;
    }
    if (!parsed || typeof parsed !== "object" || !("type" in parsed)) {
      this.emitError(new OpenAIWebSocketTransportError({
        message: `OpenAIWebSocketManager: unexpected message shape: ${text.slice(0, 200)}`,
      }));
      return;
    }

    const event = parsed as OpenAIWebSocketEvent;
    const response = (event as { response?: unknown }).response;
    if (event.type === "response.completed" && isResponseObject(response)) {
      this._previousResponseId = response.id;
    }
    this.emit("message", event);
  }
}
