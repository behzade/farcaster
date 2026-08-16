import assert from "node:assert/strict";
import { existsSync, mkdtempSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import type { BrokerExecRequest, BrokerExecResult } from "./broker-client.ts";
import { DEFAULT_CONFIG } from "./sandbox-config.ts";
import { formatDenialSummary } from "./denial-summary.ts";
import { createNativeSandboxOps, type NativeBroker } from "./native-sandbox-ops.ts";

class FakeBroker implements NativeBroker {
	readonly requests: BrokerExecRequest[] = [];
	readonly result: BrokerExecResult;
	constructor(result: BrokerExecResult) {
		this.result = result;
	}
	async exec(request: BrokerExecRequest, onData: (data: Buffer) => void): Promise<BrokerExecResult> {
		this.requests.push(request);
		onData(Buffer.from("command failed\n"));
		return this.result;
	}
}

test("one failed command makes exactly one broker request and returns a bounded grouped denial", async () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-one-run-denial-"));
	const paths = Array.from({ length: 20 }, (_, index) => `/external/state/file-${index}.db`);
	const broker = new FakeBroker({
		exitCode: 1,
		denials: paths.map((path) => ({ operation: "file-write-create", path, process: "tool" })),
		denialsComplete: false,
	});
	const output: Buffer[] = [];
	const operations = createNativeSandboxOps(broker, DEFAULT_CONFIG, [], [], "tool-one-run");
	const result = await operations.exec("failing-tool", cwd, { onData: (data) => output.push(data) });
	const text = Buffer.concat(output).toString("utf8");
	assert.equal(result.exitCode, 1);
	assert.equal(broker.requests.length, 1);
	assert.equal(broker.requests[0]?.id, "tool-one-run");
	assert.match(text, /Sandbox reported 20 denial hints/);
	assert.match(text, /write access: 20 under \/external\/state/);
	assert.equal((text.match(/  example:/g) ?? []).length, 3);
	assert.match(text, /Use request_access/);
	assert.match(text, /No command was retried/);
	assert.doesNotMatch(text, /Retrying command|Allow once/);
});

test("known host development caches recommend the managed cache adapter", async () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-cache-denial-"));
	const broker = new FakeBroker({
		exitCode: 1,
		denials: [{
			operation: "file-write-create",
			path: join(homedir(), ".cargo", "registry", "cache.db"),
			process: "cargo",
		}],
		denialsComplete: true,
	});
	const output: Buffer[] = [];
	await createNativeSandboxOps(broker, DEFAULT_CONFIG, [], [], "cache-denial").exec("cargo build", cwd, {
		onData: (data) => output.push(data),
	});
	const text = Buffer.concat(output).toString("utf8");
	assert.match(text, /host development cache \(Cargo\)/);
	assert.match(text, /development_cache environment mapping/);
	assert.doesNotMatch(text, /smallest portable file\/tree/);
	assert.equal(broker.requests.length, 1);
});

test("network-only and mixed denial hints stay grouped with three total examples", async () => {
	const networkOnly = formatDenialSummary([
		{ operation: "network-outbound", path: null, process: "curl" },
	], false);
	assert.match(networkOnly ?? "", /network access: 1/);
	assert.match(networkOnly ?? "", /example: process curl/);
	assert.match(networkOnly ?? "", /exact network host, or network_local/);

	const cwd = mkdtempSync(join(tmpdir(), "pi-mixed-denial-"));
	const broker = new FakeBroker({
		exitCode: 1,
		denials: [
			{ operation: "file-read-data", path: "/dev/null", process: "cat" },
			{ operation: "file-write-create", path: join(homedir(), ".npm", "cache", "a"), process: "npm" },
			{ operation: "file-write-create", path: "/external/state/a", process: "tool" },
			{ operation: "network-outbound", path: "api.example.com:443", process: "curl" },
			{ operation: "network-bind", path: "127.0.0.1:3000", process: "server" },
		],
		denialsComplete: false,
	});
	const output: Buffer[] = [];
	await createNativeSandboxOps(broker, DEFAULT_CONFIG, [], [], "mixed-denial").exec("tool", cwd, {
		onData: (data) => output.push(data),
	});
	const text = Buffer.concat(output).toString("utf8");
	assert.match(text, /Sandbox reported 4 denial hints/);
	assert.match(text, /host development cache \(npm\)/);
	assert.match(text, /write access/);
	assert.match(text, /network access: 2/);
	assert.match(text, /development_cache environment mapping/);
	assert.match(text, /smallest portable file\/tree, exact network host, or network_local/);
	assert.equal((text.match(/  example:/g) ?? []).length, 3);
	assert.doesNotMatch(text, /\/dev\/null/);
	assert.equal(broker.requests.length, 1);
});

test("interruption closes the command-scoped network proxy", async () => {
	let request: BrokerExecRequest | undefined;
	let startedResolve!: () => void;
	const started = new Promise<void>((resolve) => {
		startedResolve = resolve;
	});
	const broker: NativeBroker = {
		exec(next, _onData, signal) {
			request = next;
			startedResolve();
			return new Promise((_resolve, reject) => {
				signal?.addEventListener("abort", () => reject(new Error("aborted")), { once: true });
			});
		},
	};
	const controller = new AbortController();
	const running = createNativeSandboxOps(
		broker,
		DEFAULT_CONFIG,
		[],
		["example.com"],
		"interrupt-cleanup",
	).exec("sleep", tmpdir(), {
		onData() {},
		signal: controller.signal,
	});
	await started;
	controller.abort();
	await assert.rejects(running);

	assert.equal(request?.policy.network.mode, "proxy");
	if (request?.policy.network.mode !== "proxy") throw new Error("proxy request missing");
	assert.equal(existsSync(request.policy.network.unix_socket), false);
});

test("filesystem grants are revalidated immediately before the broker request", async () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-revalidate-grants-"));
	const broker = new FakeBroker({ exitCode: 0, denials: [], denialsComplete: true });
	const operations = createNativeSandboxOps(
		broker,
		DEFAULT_CONFIG,
		[],
		[],
		"revalidate",
		false,
		() => { throw new Error("approved path became a symlink"); },
	);
	await assert.rejects(
		operations.exec("true", cwd, { onData() {} }),
		/approved path became a symlink/,
	);
	assert.equal(broker.requests.length, 0);
});
