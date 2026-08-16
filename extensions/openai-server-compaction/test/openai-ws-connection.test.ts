import assert from "node:assert/strict";
import { EventEmitter, once } from "node:events";
import { describe, it } from "node:test";
import {
  OpenAIWebSocketManager,
  OpenAIWebSocketTransportError,
} from "../src/openai-ws-connection.ts";

class FakeWebSocket extends EventEmitter {
  readyState = 0;
  readonly sent: string[] = [];
  closeCalls = 0;
  terminateCalls = 0;

  open(): void {
    this.readyState = 1;
    this.emit("open");
  }

  send(data: string): void {
    if (this.readyState !== 1) throw new Error("not open");
    this.sent.push(data);
  }

  close(code = 1000, reason = ""): void {
    this.closeCalls++;
    this.readyState = 3;
    this.emit("close", code, Buffer.from(reason));
  }

  terminate(): void {
    this.terminateCalls++;
    this.readyState = 3;
    this.emit("close", 1006, Buffer.from("terminated"));
  }
}

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

async function waitForListener(socket: FakeWebSocket, event: string): Promise<void> {
  for (let attempt = 0; attempt < 20 && socket.listenerCount(event) === 0; attempt++) {
    await tick();
  }
  assert.ok(socket.listenerCount(event) > 0, `${event} listener was not installed`);
}

describe("OpenAIWebSocketManager Effect resources", () => {
  it("opens, dispatches messages, and removes all owned socket listeners on release", async () => {
    const socket = new FakeWebSocket();
    const manager = new OpenAIWebSocketManager({ socketFactory: () => socket });
    const messages: unknown[] = [];
    manager.onMessage((event) => messages.push(event));

    const connecting = manager.connect("key");
    await waitForListener(socket, "open");
    socket.open();
    await connecting;
    socket.emit("message", JSON.stringify({
      type: "response.completed",
      response: { id: "resp_1", output: [], object: "response", created_at: 0, status: "completed", model: "gpt" },
    }));
    await tick();

    assert.equal(manager.isConnected(), true);
    assert.equal(manager.previousResponseId, "resp_1");
    assert.equal(messages.length, 1);
    manager.close();
    assert.equal(socket.listenerCount("open"), 0);
    assert.equal(socket.listenerCount("error"), 0);
    assert.equal(socket.listenerCount("close"), 0);
    assert.equal(socket.listenerCount("message"), 0);
  });

  it("reports an open failure as a typed transport error and cleans listeners", async () => {
    const socket = new FakeWebSocket();
    const manager = new OpenAIWebSocketManager({
      maxRetries: 0,
      socketFactory: () => socket,
    });
    const connecting = manager.connect("key");
    const settled = connecting.then(
      () => undefined,
      (error: unknown) => error,
    );
    await waitForListener(socket, "error");
    socket.emit("error", new Error("handshake rejected"));

    const failure = await settled;
    assert.ok(failure instanceof OpenAIWebSocketTransportError);
    manager.close();
    assert.equal(socket.listenerCount("error"), 0);
    assert.equal(socket.listenerCount("close"), 0);
  });

  it("uses the configured reconnect count and backoff sequence without broader retries", async () => {
    const sockets: FakeWebSocket[] = [];
    const attempts: number[] = [];
    const manager = new OpenAIWebSocketManager({
      maxRetries: 2,
      backoffDelaysMs: [5, 10],
      socketFactory: () => {
        const socket = new FakeWebSocket();
        sockets.push(socket);
        attempts.push(Date.now());
        queueMicrotask(() => {
          if (sockets.length === 1) socket.open();
          else socket.emit("close", 1006, Buffer.from("retry failed"));
        });
        return socket;
      },
    });
    const maxError = once(manager, "error");
    await manager.connect("key");
    sockets[0]!.close(1006, "lost");
    const [error] = await maxError;

    assert.match((error as Error).message, /max reconnect retries \(2\) exceeded/);
    assert.equal(sockets.length, 3);
    assert.ok((attempts[1]! - attempts[0]!) >= 4);
    assert.ok((attempts[2]! - attempts[1]!) >= 9);
    manager.close();
    for (const socket of sockets) assert.equal(socket.listenerCount("message"), 0);
  });

  it("never drops a burst of socket messages from its unbounded queue", async () => {
    const socket = new FakeWebSocket();
    const manager = new OpenAIWebSocketManager({ socketFactory: () => socket });
    const seen: number[] = [];
    manager.onMessage((event) => seen.push((event as { index: number }).index));
    const connecting = manager.connect("key");
    await waitForListener(socket, "open");
    socket.open();
    await connecting;
    for (let index = 0; index < 500; index++) {
      socket.emit("message", JSON.stringify({ type: "test", index }));
    }
    await tick();
    assert.deepEqual(seen, Array.from({ length: 500 }, (_, index) => index));
    manager.close();
  });
});
