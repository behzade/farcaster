import assert from "node:assert/strict";
import { chmodSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
	FramedJsonDecoder,
	MAX_BROKER_FRAME_BYTES,
	SandboxBrokerClient,
	encodeBrokerFrame,
	isSupportedReadyEvent,
	validateBrokerEvent,
	type BrokerExecRequest,
} from "./broker-client.ts";

function request(cwd: string, id = "command-1"): BrokerExecRequest {
	return {
		type: "exec",
		id,
		command: { program: "/bin/true", args: [] },
		cwd,
		env: { PATH: "/usr/bin:/bin" },
		timeout_ms: 1000,
		policy: {
			base_rights: [],
			grants: [],
			denies: [],
			network: { mode: "blocked" },
			unix_socket_roots: [],
			output_limit_bytes: 1024,
		},
	};
}

test("framed JSON survives split and joined chunks", () => {
	const first = encodeBrokerFrame({ type: "one", text: "line one\nline two" });
	const second = encodeBrokerFrame({ type: "two" });
	const bytes = Buffer.concat([first, second]);
	const decoder = new FramedJsonDecoder();
	assert.deepEqual(decoder.push(bytes.subarray(0, 3)), []);
	assert.deepEqual(decoder.push(bytes.subarray(3)), [
		{ type: "one", text: "line one\nline two" },
		{ type: "two" },
	]);
	decoder.finish();
});

test("framing rejects partial, oversized, and malformed UTF-8 input", () => {
	const partial = new FramedJsonDecoder();
	partial.push(Buffer.from([0, 0]));
	assert.throws(() => partial.finish(), /partial frame/);

	const oversized = Buffer.alloc(4);
	oversized.writeUInt32BE(MAX_BROKER_FRAME_BYTES + 1);
	assert.throws(() => new FramedJsonDecoder().push(oversized), /exceeds/);

	const malformed = Buffer.from([0, 0, 0, 2, 0xc3, 0x28]);
	assert.throws(() => new FramedJsonDecoder().push(malformed));
});

test("readiness accepts only the platform's fixed native backend", () => {
	const ready = (platform: string, backend: string) => ({
		type: "ready" as const,
		version: 4,
		platform,
		backend,
		can_exec: true,
		max_frame_bytes: MAX_BROKER_FRAME_BYTES,
	});
	assert.equal(isSupportedReadyEvent(ready("macos", "seatbelt"), "darwin"), true);
	assert.equal(isSupportedReadyEvent(ready("linux", "bubblewrap"), "linux"), true);
	assert.equal(isSupportedReadyEvent(ready("linux", "seatbelt"), "linux"), false);
	assert.equal(isSupportedReadyEvent(ready("macos", "bubblewrap"), "darwin"), false);
	assert.equal(isSupportedReadyEvent(ready("linux", "bubblewrap"), "darwin"), false);
	assert.equal(isSupportedReadyEvent(ready("linux", "bubblewrap"), "win32"), false);
	assert.equal(
		isSupportedReadyEvent({ ...ready("macos", "seatbelt"), version: 3 }, "darwin"),
		false,
	);
	assert.equal(
		isSupportedReadyEvent({ ...ready("linux", "bubblewrap"), can_exec: false }, "linux"),
		false,
	);
});

test("event validation rejects unknown fields and non-canonical output", () => {
	assert.throws(
		() =>
			validateBrokerEvent({
				type: "ready",
				version: 4,
				platform: "macos",
				backend: "seatbelt",
				can_exec: true,
				max_frame_bytes: MAX_BROKER_FRAME_BYTES,
				extra: true,
			}),
		/fields are invalid/,
	);
	assert.throws(
		() =>
			validateBrokerEvent({
				type: "stdout",
				id: "one",
				sequence: 0,
				data_base64: "not base64",
			}),
		/data_base64 is invalid/,
	);
	assert.throws(
		() =>
			validateBrokerEvent({
				type: "error",
				id: "one",
				code: "made_up",
				message: "bad",
			}),
		/error.code is invalid/,
	);
});

test("client requires readiness and streams typed command output", async () => {
	const directory = mkdtempSync(join(tmpdir(), "pi-fake-broker-"));
	const broker = join(directory, "broker");
	writeFileSync(
		broker,
		`#!/usr/bin/env node
const encode = value => {
  const body = Buffer.from(JSON.stringify(value));
  const frame = Buffer.alloc(body.length + 4);
  frame.writeUInt32BE(body.length, 0);
  body.copy(frame, 4);
  process.stdout.write(frame);
};
let pending = Buffer.alloc(0);
encode({ type: "ready", version: 4, platform: "macos", backend: "seatbelt", can_exec: true, max_frame_bytes: ${MAX_BROKER_FRAME_BYTES} });
process.stdin.on("data", chunk => {
  pending = Buffer.concat([pending, chunk]);
  while (pending.length >= 4) {
    const size = pending.readUInt32BE(0);
    if (pending.length < size + 4) return;
    const message = JSON.parse(pending.subarray(4, size + 4));
    pending = pending.subarray(size + 4);
    if (message.type === "exec") {
      encode({ type: "started", id: message.id, pid: process.pid });
      encode({ type: "stdout", id: message.id, sequence: 0, data_base64: Buffer.from(process.env.PI_SANDBOX_DEVELOPMENT_CACHE_ROOT + "\\n").toString("base64") });
      encode({ type: "denials", id: message.id, items: [{ operation: "file-write-create", path: "/state/file", process: "tool" }], complete: false });
      encode({ type: "exit", id: message.id, code: 0, signal: null, timed_out: false, cancelled: false, output_truncated: false });
    } else if (message.type === "shutdown") {
      process.exit(0);
    }
  }
});
`,
	);
	chmodSync(broker, 0o700);

	const client = await SandboxBrokerClient.start(broker, "darwin", "/tmp/cache-root");
	const output: Buffer[] = [];
	assert.deepEqual(await client.exec(request(directory), (chunk) => output.push(chunk)), {
		exitCode: 0,
		denials: [
			{
				operation: "file-write-create",
				path: "/state/file",
				process: "tool",
			},
		],
		denialsComplete: false,
	});
	assert.equal(Buffer.concat(output).toString("utf8"), "/tmp/cache-root\n");
	await client.shutdown();
});

test("Linux client accepts exit without macOS denial hints", async () => {
	const directory = mkdtempSync(join(tmpdir(), "pi-fake-linux-broker-"));
	const broker = join(directory, "broker");
	writeFileSync(
		broker,
		`#!/usr/bin/env node
const encode = value => {
  const body = Buffer.from(JSON.stringify(value));
  const frame = Buffer.alloc(body.length + 4);
  frame.writeUInt32BE(body.length, 0);
  body.copy(frame, 4);
  process.stdout.write(frame);
};
let pending = Buffer.alloc(0);
encode({ type: "ready", version: 4, platform: "linux", backend: "bubblewrap", can_exec: true, max_frame_bytes: ${MAX_BROKER_FRAME_BYTES} });
process.stdin.on("data", chunk => {
  pending = Buffer.concat([pending, chunk]);
  if (pending.length < 4) return;
  const size = pending.readUInt32BE(0);
  if (pending.length < size + 4) return;
  const message = JSON.parse(pending.subarray(4, size + 4));
  pending = pending.subarray(size + 4);
  if (message.type === "exec") {
    encode({ type: "started", id: message.id, pid: process.pid });
    encode({ type: "exit", id: message.id, code: 0, signal: null, timed_out: false, cancelled: false, output_truncated: false });
  } else if (message.type === "shutdown") {
    process.exit(0);
  }
});
`,
	);
	chmodSync(broker, 0o700);

	const client = await SandboxBrokerClient.start(broker, "linux");
	assert.deepEqual(await client.exec(request(directory), () => {}), {
		exitCode: 0,
		denials: [],
		denialsComplete: false,
	});
	await client.shutdown();
});

test("client rejects a pre-start error after started", async () => {
	const directory = mkdtempSync(join(tmpdir(), "pi-fake-broker-state-"));
	const broker = join(directory, "broker");
	writeFileSync(
		broker,
		`#!/usr/bin/env node
const encode = value => {
  const body = Buffer.from(JSON.stringify(value));
  const frame = Buffer.alloc(body.length + 4);
  frame.writeUInt32BE(body.length, 0);
  body.copy(frame, 4);
  process.stdout.write(frame);
};
let pending = Buffer.alloc(0);
encode({ type: "ready", version: 4, platform: "macos", backend: "seatbelt", can_exec: true, max_frame_bytes: ${MAX_BROKER_FRAME_BYTES} });
process.stdin.on("data", chunk => {
  pending = Buffer.concat([pending, chunk]);
  if (pending.length < 4) return;
  const size = pending.readUInt32BE(0);
  if (pending.length < size + 4) return;
  const message = JSON.parse(pending.subarray(4, size + 4));
  if (message.type === "exec") {
    encode({ type: "started", id: message.id, pid: process.pid });
    encode({ type: "error", id: message.id, code: "protocol_error", message: "late" });
  }
});
`,
	);
	chmodSync(broker, 0o700);

	const client = await SandboxBrokerClient.start(broker, "darwin");
	await assert.rejects(
		client.exec(request(directory), () => {}),
		/pre-start error after starting command/,
	);
	await client.shutdown();
});

test("readiness failure finalizes the spawned broker", async () => {
	const directory = mkdtempSync(join(tmpdir(), "pi-fake-broker-readiness-failure-"));
	const broker = join(directory, "broker");
	const pidFile = join(directory, "pid");
	writeFileSync(
		broker,
		`#!/usr/bin/env node
const fs = require("node:fs");
fs.writeFileSync(${JSON.stringify(pidFile)}, String(process.pid));
const body = Buffer.from(JSON.stringify({ type: "ready", version: 999, platform: "macos", backend: "seatbelt", can_exec: true, max_frame_bytes: ${MAX_BROKER_FRAME_BYTES} }));
const frame = Buffer.alloc(body.length + 4); frame.writeUInt32BE(body.length, 0); body.copy(frame, 4); process.stdout.write(frame);
setInterval(() => {}, 1000);
`,
	);
	chmodSync(broker, 0o700);
	await assert.rejects(SandboxBrokerClient.start(broker, "darwin"), /Unsupported sandbox broker/);
	const pid = Number(await import("node:fs/promises").then((fs) => fs.readFile(pidFile, "utf8")));
	await assert.rejects(async () => {
		for (let attempt = 0; attempt < 50; attempt += 1) {
			try { process.kill(pid, 0); } catch { throw new Error("finalized"); }
			await new Promise((resolve) => setTimeout(resolve, 10));
		}
	}, /finalized/);
});

test("command abort emits exactly one cancel and removes its abort listener", async () => {
	const directory = mkdtempSync(join(tmpdir(), "pi-fake-broker-abort-"));
	const broker = join(directory, "broker");
	const log = join(directory, "messages");
	writeFileSync(
		broker,
		`#!/usr/bin/env node
const fs = require("node:fs");
const encode = value => { const body = Buffer.from(JSON.stringify(value)); const frame = Buffer.alloc(body.length + 4); frame.writeUInt32BE(body.length, 0); body.copy(frame, 4); process.stdout.write(frame); };
encode({ type: "ready", version: 4, platform: "linux", backend: "bubblewrap", can_exec: true, max_frame_bytes: ${MAX_BROKER_FRAME_BYTES} });
let pending = Buffer.alloc(0);
process.stdin.on("data", chunk => { pending = Buffer.concat([pending, chunk]); while (pending.length >= 4) { const size = pending.readUInt32BE(0); if (pending.length < size + 4) return; const message = JSON.parse(pending.subarray(4, size + 4)); pending = pending.subarray(size + 4); fs.appendFileSync(${JSON.stringify(log)}, message.type + "\\n"); if (message.type === "exec") encode({ type: "started", id: message.id, pid: process.pid }); else if (message.type === "cancel") encode({ type: "exit", id: message.id, code: null, signal: 15, timed_out: false, cancelled: true, output_truncated: false }); else if (message.type === "shutdown") process.exit(0); } });
`,
	);
	chmodSync(broker, 0o700);
	const client = await SandboxBrokerClient.start(broker, "linux");
	const controller = new AbortController();
	const executing = client.exec(request(directory), () => {}, controller.signal);
	await new Promise((resolve) => setTimeout(resolve, 20));
	controller.abort();
	controller.abort();
	await assert.rejects(executing, /aborted|interrupt/i);
	await client.shutdown();
	const messages = await import("node:fs/promises").then((fs) => fs.readFile(log, "utf8"));
	assert.equal(messages.split("\n").filter((line) => line === "exec").length, 1);
	assert.equal(messages.split("\n").filter((line) => line === "cancel").length, 1);
});

test("client requires denial hints before a started command exits", async () => {
	const directory = mkdtempSync(join(tmpdir(), "pi-fake-broker-denials-"));
	const broker = join(directory, "broker");
	writeFileSync(
		broker,
		`#!/usr/bin/env node
const encode = value => {
  const body = Buffer.from(JSON.stringify(value));
  const frame = Buffer.alloc(body.length + 4);
  frame.writeUInt32BE(body.length, 0);
  body.copy(frame, 4);
  process.stdout.write(frame);
};
let pending = Buffer.alloc(0);
encode({ type: "ready", version: 4, platform: "macos", backend: "seatbelt", can_exec: true, max_frame_bytes: ${MAX_BROKER_FRAME_BYTES} });
process.stdin.on("data", chunk => {
  pending = Buffer.concat([pending, chunk]);
  if (pending.length < 4) return;
  const size = pending.readUInt32BE(0);
  if (pending.length < size + 4) return;
  const message = JSON.parse(pending.subarray(4, size + 4));
  if (message.type === "exec") {
    encode({ type: "started", id: message.id, pid: process.pid });
    encode({ type: "exit", id: message.id, code: 1, signal: null, timed_out: false, cancelled: false, output_truncated: false });
  }
});
`,
	);
	chmodSync(broker, 0o700);

	const client = await SandboxBrokerClient.start(broker, "darwin");
	await assert.rejects(
		client.exec(request(directory), () => {}),
		/exit arrived before denials/,
	);
	await client.shutdown();
});
