import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import type {
	BrokerExecRequest,
	BrokerExecResult,
} from "./broker-client.ts";
import { DEFAULT_CONFIG } from "./codex-command.ts";
import {
	createApprovingNativeSandboxOps,
	type NativeBroker,
} from "./native-sandbox-ops.ts";

class FakeBroker implements NativeBroker {
	readonly requests: BrokerExecRequest[] = [];
	readonly run: (
		request: BrokerExecRequest,
		onData: (data: Buffer) => void,
	) => BrokerExecResult;

	constructor(
		run: (
			request: BrokerExecRequest,
			onData: (data: Buffer) => void,
		) => BrokerExecResult,
	) {
		this.run = run;
	}

	async exec(
		request: BrokerExecRequest,
		onData: (data: Buffer) => void,
	): Promise<BrokerExecResult> {
		this.requests.push(request);
		return this.run(request, onData);
	}
}

function failed(path: string): BrokerExecResult {
	return {
		exitCode: 1,
		denials: [
			{
				operation: "file-write-create",
				path,
				process: "issues",
			},
		],
		denialsComplete: false,
	};
}

test("native structured denial prompts and retries without parsing app output", async () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-native-retry-"));
	const path = join(homedir(), `pi-native-retry-${process.pid}`, "state.db");
	const broker = new FakeBroker((request, onData) => {
		if (request.id.endsWith("attempt-0")) {
			onData(Buffer.from("service unavailable\n"));
			return failed(path);
		}
		return { exitCode: 0, denials: [], denialsComplete: false };
	});
	const approvals: string[] = [];
	const output: Buffer[] = [];
	const operations = createApprovingNativeSandboxOps({
		client: broker,
		config: DEFAULT_CONFIG,
		initialPermissions: [],
		toolCallId: "tool-1",
		blockedPaths: [],
		async approve(permission) {
			approvals.push(permission.path);
			return true;
		},
	});

	const result = await operations.exec("issues search", cwd, {
		onData: (data) => output.push(data),
	});
	assert.equal(result.exitCode, 0);
	assert.deepEqual(approvals, [path]);
	assert.deepEqual(
		broker.requests.map((request) => request.id),
		["tool-1/attempt-0", "tool-1/attempt-1"],
	);
	assert.equal(broker.requests[0]?.policy.grants.length, 0);
	assert.deepEqual(broker.requests[1]?.policy.grants, [
		{
			access: "write",
			path,
			scope: "file",
			missing_path: "create_file",
		},
	]);
	assert.equal(
		Buffer.concat(output).toString("utf8"),
		"service unavailable\n\n[Retrying command with approved IO rights]\n",
	);
});

test("empty incomplete denial results do not prompt or retry", async () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-native-no-retry-"));
	const broker = new FakeBroker(() => ({
		exitCode: 1,
		denials: [],
		denialsComplete: false,
	}));
	let prompts = 0;
	const operations = createApprovingNativeSandboxOps({
		client: broker,
		config: DEFAULT_CONFIG,
		initialPermissions: [],
		toolCallId: "tool-2",
		blockedPaths: [],
		async approve() {
			prompts += 1;
			return true;
		},
	});

	const result = await operations.exec("false", cwd, { onData() {} });
	assert.equal(result.exitCode, 1);
	assert.equal(prompts, 0);
	assert.equal(broker.requests.length, 1);
});

test("cancellation during native approval prevents retry", async () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-native-abort-"));
	const path = join(homedir(), `pi-native-abort-${process.pid}`, "state.db");
	const broker = new FakeBroker(() => failed(path));
	const controller = new AbortController();
	let approvalStarted: (() => void) | undefined;
	const started = new Promise<void>((resolve) => {
		approvalStarted = resolve;
	});
	const operations = createApprovingNativeSandboxOps({
		client: broker,
		config: DEFAULT_CONFIG,
		initialPermissions: [],
		toolCallId: "tool-3",
		blockedPaths: [],
		approve(_permission, signal) {
			approvalStarted?.();
			return new Promise<boolean>((resolve) => {
				if (signal?.aborted) resolve(false);
				else signal?.addEventListener("abort", () => resolve(false), { once: true });
			});
		},
	});

	const running = operations.exec("issues search", cwd, {
		onData() {},
		signal: controller.signal,
	});
	await started;
	controller.abort();
	await assert.rejects(running, /aborted/);
	assert.equal(broker.requests.length, 1);
});
