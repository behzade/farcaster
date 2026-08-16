import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { afterEach, describe, it } from "node:test";
import { Effect } from "effect";
import type { AssistantMessageEvent } from "@earendil-works/pi-ai";
import {
  createOpenAIWebSocketStreamFn,
  releaseAllWsSessions,
  releaseWsSession,
} from "../src/openai-ws-stream.ts";

class FakeWebSocket extends EventEmitter {
  readyState = 0;
  readonly sent: Record<string, unknown>[] = [];
  onSend?: (body: Record<string, unknown>) => void;
  throwAfterSend = false;

  open(): void {
    this.readyState = 1;
    this.emit("open");
  }

  send(data: string): void {
    const body = JSON.parse(data) as Record<string, unknown>;
    this.sent.push(body);
    this.onSend?.(body);
    if (this.throwAfterSend) throw new Error("send outcome is ambiguous");
  }

  close(code = 1000, reason = ""): void {
    this.readyState = 3;
    this.emit("close", code, Buffer.from(reason));
  }

  terminate(): void {
    this.close(1006, "terminated");
  }
}

const model = {
  id: "gpt-5.4",
  name: "GPT",
  api: "openai-responses",
  provider: "openai",
  baseUrl: "https://api.openai.com/v1",
  reasoning: false,
  contextWindow: 100_000,
  maxTokens: 4096,
  cost: { input: 1, output: 2, cacheRead: 0.5, cacheWrite: 1, total: 0 },
} as never;

const context = {
  systemPrompt: "system",
  messages: [{ role: "user", content: "hello", timestamp: Date.now() }],
  tools: [],
} as never;

function completed(id = "resp_1") {
  return {
    type: "response.completed",
    response: {
      id,
      object: "response",
      created_at: 1,
      status: "completed",
      model: "gpt-5.4",
      output: [{
        type: "message",
        id: "msg_1",
        role: "assistant",
        content: [{ type: "output_text", text: "hello world" }],
      }],
      usage: {
        input_tokens: 10,
        output_tokens: 2,
        total_tokens: 12,
        input_tokens_details: { cached_tokens: 4 },
      },
    },
  };
}

async function collect(stream: AsyncIterable<AssistantMessageEvent>): Promise<AssistantMessageEvent[]> {
  const events: AssistantMessageEvent[] = [];
  for await (const event of stream) events.push(event);
  return events;
}

const nextTurn = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

describe("OpenAI WebSocket stream Effect orchestration", () => {
  afterEach(() => releaseAllWsSessions());

  it("preserves start/text-start/delta/done ordering, response id, usage, and burst delivery", async () => {
    const socket = new FakeWebSocket();
    socket.onSend = () => queueMicrotask(() => {
      for (const delta of ["hello", " ", "world"]) {
        socket.emit("message", JSON.stringify({ type: "response.output_text.delta", delta }));
      }
      socket.emit("message", JSON.stringify(completed()));
    });
    const streamFn = createOpenAIWebSocketStreamFn({
      maxRetries: 0,
      socketFactory: () => {
        queueMicrotask(() => socket.open());
        return socket;
      },
    });

    const events = await collect(streamFn(model, context, {
      apiKey: "key",
      sessionId: "ordering",
      transport: "websocket",
    } as never));

    assert.deepEqual(events.map((event) => event.type), [
      "start", "text_start", "text_delta", "text_delta", "text_delta", "done",
    ]);
    const done = events.at(-1) as Extract<AssistantMessageEvent, { type: "done" }>;
    assert.equal(done.message.responseId, "resp_1");
    assert.equal(done.message.usage.input, 6);
    assert.equal(done.message.usage.cacheRead, 4);
    releaseWsSession("ordering");
  });

  it("treats warm-up failure as best effort and removes warm-up listeners before the request", async () => {
    const socket = new FakeWebSocket();
    socket.onSend = (body) => queueMicrotask(() => {
      if (body.generate === false) {
        socket.emit("message", JSON.stringify({
          type: "response.failed",
          response: { ...completed("warm").response, status: "failed", error: { code: "warm", message: "no warm" } },
        }));
      } else {
        socket.emit("message", JSON.stringify(completed("actual")));
      }
    });
    const streamFn = createOpenAIWebSocketStreamFn({ socketFactory: () => {
      queueMicrotask(() => socket.open());
      return socket;
    } });
    const events = await collect(streamFn(model, context, {
      apiKey: "key",
      sessionId: "warmup",
      transport: "websocket",
      openaiWsWarmup: true,
    } as never));

    assert.deepEqual(socket.sent.map((body) => body.generate), [false, undefined]);
    assert.equal(events.at(-1)?.type, "done");
    assert.equal(socket.listenerCount("message"), 1, "only the manager's owned listener remains");
    releaseWsSession("warmup");
  });

  it("turns a close during a request into one error completion and releases the session", async () => {
    const socket = new FakeWebSocket();
    socket.onSend = () => queueMicrotask(() => socket.close(1006, "lost"));
    const streamFn = createOpenAIWebSocketStreamFn({ socketFactory: () => {
      queueMicrotask(() => socket.open());
      return socket;
    } });
    const events = await collect(streamFn(model, context, {
      apiKey: "key",
      sessionId: "close-mid-request",
      transport: "websocket",
    } as never));

    assert.deepEqual(events.map((event) => event.type), ["start", "error"]);
    assert.match((events[1] as Extract<AssistantMessageEvent, { type: "error" }>).error.errorMessage ?? "", /closed mid-request/);
    assert.equal(socket.listenerCount("message"), 0);
    assert.equal(socket.listenerCount("close"), 0);
  });

  it("cleans abort, manager, and socket listeners on abort", async () => {
    const socket = new FakeWebSocket();
    const controller = new AbortController();
    const streamFn = createOpenAIWebSocketStreamFn({ socketFactory: () => {
      queueMicrotask(() => socket.open());
      return socket;
    } });
    const collecting = collect(streamFn(model, context, {
      apiKey: "key",
      sessionId: "abort",
      transport: "websocket",
      signal: controller.signal,
    } as never));
    await nextTurn();
    controller.abort();
    const events = await collecting;

    assert.deepEqual(events.map((event) => event.type), ["start", "error"]);
    assert.equal((events[1] as Extract<AssistantMessageEvent, { type: "error" }>).reason, "aborted");
    assert.equal(socket.listenerCount("message"), 0);
    assert.equal(socket.listenerCount("close"), 0);
  });

  it("does not send a pre-aborted request", async () => {
    const socket = new FakeWebSocket();
    const controller = new AbortController();
    controller.abort();
    const streamFn = createOpenAIWebSocketStreamFn({ socketFactory: () => {
      queueMicrotask(() => socket.open());
      return socket;
    } });
    const events = await collect(streamFn(model, context, {
      apiKey: "key",
      sessionId: "pre-aborted",
      transport: "websocket",
      signal: controller.signal,
    } as never));

    assert.equal(socket.sent.length, 0);
    assert.deepEqual(events.map((event) => event.type), ["error"]);
    assert.equal((events[0] as Extract<AssistantMessageEvent, { type: "error" }>).reason, "aborted");
  });

  it("uses HTTP fallback only for auto transport connection/send failures", async () => {
    let fallbackCalls = 0;
    const fallback = (_model: unknown, _context: unknown, _options: unknown, eventStream: { push(event: AssistantMessageEvent): void }) =>
      Effect.sync(() => {
        fallbackCalls++;
        eventStream.push({ type: "start", partial: {} as never });
        eventStream.push({ type: "done", reason: "stop", message: { stopReason: "stop" } as never });
      });
    const managerOptions = {
      maxRetries: 0,
      socketFactory: () => { throw new Error("offline"); },
    };
    const auto = createOpenAIWebSocketStreamFn(managerOptions, { httpFallback: fallback as never });
    const autoEvents = await collect(auto(model, context, {
      apiKey: "key",
      sessionId: "fallback-auto",
      transport: "auto",
    } as never));
    const explicit = createOpenAIWebSocketStreamFn(managerOptions, { httpFallback: fallback as never });
    const explicitEvents = await collect(explicit(model, context, {
      apiKey: "key",
      sessionId: "fallback-explicit",
      transport: "websocket",
    } as never));

    assert.equal(fallbackCalls, 1);
    assert.deepEqual(autoEvents.map((event) => event.type), ["start", "done"]);
    assert.deepEqual(explicitEvents.map((event) => event.type), ["error"]);
  });

  it("does not fall back after a send attempt with an ambiguous outcome", async () => {
    const socket = new FakeWebSocket();
    socket.throwAfterSend = true;
    let fallbackCalls = 0;
    const fallback = () => Effect.sync(() => { fallbackCalls++; });
    const streamFn = createOpenAIWebSocketStreamFn({ socketFactory: () => {
      queueMicrotask(() => socket.open());
      return socket;
    } }, { httpFallback: fallback as never });
    const events = await collect(streamFn(model, context, {
      apiKey: "key",
      sessionId: "ambiguous-send",
      transport: "auto",
    } as never));

    assert.equal(socket.sent.length, 1);
    assert.equal(fallbackCalls, 0);
    assert.deepEqual(events.map((event) => event.type), ["error"]);
  });

  it("serializes concurrent requests sharing one session", async () => {
    const socket = new FakeWebSocket();
    const streamFn = createOpenAIWebSocketStreamFn({ socketFactory: () => {
      queueMicrotask(() => socket.open());
      return socket;
    } });
    const first = collect(streamFn(model, context, {
      apiKey: "key",
      sessionId: "serialized",
      transport: "websocket",
    } as never));
    while (socket.sent.length < 1) await nextTurn();
    const second = collect(streamFn(model, context, {
      apiKey: "key",
      sessionId: "serialized",
      transport: "websocket",
    } as never));
    await nextTurn();
    assert.equal(socket.sent.length, 1);

    socket.emit("message", JSON.stringify(completed("first")));
    const firstEvents = await first;
    while (socket.sent.length < 2) await nextTurn();
    socket.emit("message", JSON.stringify(completed("second")));
    const secondEvents = await second;

    assert.equal((firstEvents.at(-1) as Extract<AssistantMessageEvent, { type: "done" }>).message.responseId, "first");
    assert.equal((secondEvents.at(-1) as Extract<AssistantMessageEvent, { type: "done" }>).message.responseId, "second");
  });

  it("keeps a completed session alive for reuse and finalizes it on explicit release", async () => {
    const socket = new FakeWebSocket();
    socket.onSend = () => queueMicrotask(() => socket.emit("message", JSON.stringify(completed("release"))));
    let acquisitions = 0;
    const streamFn = createOpenAIWebSocketStreamFn({ socketFactory: () => {
      acquisitions++;
      queueMicrotask(() => socket.open());
      return socket;
    } });
    const events = await collect(streamFn(model, context, {
      apiKey: "key",
      sessionId: "release",
      transport: "websocket",
    } as never));
    assert.equal(events.at(-1)?.type, "done");
    assert.equal(acquisitions, 1);
    assert.ok(socket.listenerCount("message") > 0);

    releaseWsSession("release");
    assert.equal(socket.listenerCount("message"), 0);
    assert.equal(socket.listenerCount("close"), 0);
  });
});
