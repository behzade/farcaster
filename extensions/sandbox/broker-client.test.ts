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
		version: 1,
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
		isSupportedReadyEvent({ ...ready("linux", "bubblewrap"), can_exec: false }, "linux"),
		false,
	);
});

test("event validation rejects unknown fields and non-canonical output", () => {
	assert.throws(
		() =>
			validateBrokerEvent({
				type: "ready",
				version: 1,
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
encode({ type: "ready", version: 1, platform: "macos", backend: "seatbelt", can_exec: true, max_frame_bytes: ${MAX_BROKER_FRAME_BYTES} });
process.stdin.on("data", chunk => {
  pending = Buffer.concat([pending, chunk]);
  while (pending.length >= 4) {
    const size = pending.readUInt32BE(0);
    if (pending.length < size + 4) return;
    const message = JSON.parse(pending.subarray(4, size + 4));
    pending = pending.subarray(size + 4);
    if (message.type === "exec") {
      encode({ type: "started", id: message.id, pid: process.pid });
      encode({ type: "stdout", id: message.id, sequence: 0, data_base64: Buffer.from("ok\\n").toString("base64") });
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

	const client = await SandboxBrokerClient.start(broker, "darwin");
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
	assert.equal(Buffer.concat(output).toString("utf8"), "ok\n");
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
encode({ type: "ready", version: 1, platform: "linux", backend: "bubblewrap", can_exec: true, max_frame_bytes: ${MAX_BROKER_FRAME_BYTES} });
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
encode({ type: "ready", version: 1, platform: "macos", backend: "seatbelt", can_exec: true, max_frame_bytes: ${MAX_BROKER_FRAME_BYTES} });
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
encode({ type: "ready", version: 1, platform: "macos", backend: "seatbelt", can_exec: true, max_frame_bytes: ${MAX_BROKER_FRAME_BYTES} });
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
