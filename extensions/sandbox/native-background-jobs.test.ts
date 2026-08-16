import assert from "node:assert/strict";
import { chmodSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { NativeBackgroundJobs } from "./native-background-jobs.ts";
import { DEFAULT_CONFIG } from "./sandbox-config.ts";

test("native background jobs keep output and stdin inside a dedicated broker", async () => {
	const directory = mkdtempSync(join(tmpdir(), "pi-native-job-test-"));
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
const mac = process.platform === "darwin";
encode({ type: "ready", version: 4, platform: mac ? "macos" : "linux", backend: mac ? "seatbelt" : "bubblewrap", can_exec: true, max_frame_bytes: 1048576 });
let pending = Buffer.alloc(0);
let active;
let sequence = 0;
process.stdin.on("data", chunk => {
  pending = Buffer.concat([pending, chunk]);
  while (pending.length >= 4) {
    const size = pending.readUInt32BE(0);
    if (pending.length < size + 4) return;
    const message = JSON.parse(pending.subarray(4, size + 4));
    pending = pending.subarray(size + 4);
    if (message.type === "exec") {
      active = message.id;
      encode({ type: "started", id: active, pid: process.pid });
      encode({ type: "stdout", id: active, sequence: sequence++, data_base64: Buffer.from("ready\\n").toString("base64") });
    } else if (message.type === "write_stdin") {
      encode({ type: "stdout", id: active, sequence: sequence++, data_base64: message.data_base64 });
    } else if (message.type === "cancel") {
      if (mac) encode({ type: "denials", id: active, items: [], complete: false });
      encode({ type: "exit", id: active, code: null, signal: 15, timed_out: false, cancelled: true, output_truncated: false });
    } else if (message.type === "shutdown") process.exit(0);
  }
});
`,
	);
	chmodSync(broker, 0o700);
	const jobs = new NativeBackgroundJobs(broker);
	let revalidated = 0;
	try {
		assert.equal(
			await jobs.start({
				name: "pi-test",
				command: "read line",
				cwd: directory,
				config: DEFAULT_CONFIG,
				permissions: [],
				revalidatePermissions: () => {
					revalidated += 1;
					return [];
				},
				networkHosts: [],
			}),
			"started pi-test",
		);
		assert.equal(revalidated, 1);
		assert.match(jobs.status("pi-test"), /state=running/);
		await new Promise((resolve) => setTimeout(resolve, 20));
		assert.match(jobs.read("pi-test", 20), /ready/);
		assert.equal(jobs.write("pi-test", Buffer.from("hello\n")), "sent input to pi-test");
		await new Promise((resolve) => setTimeout(resolve, 20));
		assert.match(jobs.read("pi-test", 20), /hello/);
		assert.equal(await jobs.stop("pi-test"), "stopped pi-test");
		assert.equal(jobs.list(), "no background jobs");
	} finally {
		await jobs.shutdown();
	}
});
