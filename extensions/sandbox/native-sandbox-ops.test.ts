import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import type {
	BrokerExecRequest,
	BrokerExecResult,
} from "./broker-client.ts";
import { DEFAULT_CONFIG } from "./sandbox-config.ts";
import { canonicalize } from "./io-permissions.ts";
import {
	createApprovingNativeSandboxOps,
	modelVisibleApprovedRetryOutput,
	type NativeApprovalRequest,
	type NativeBroker,
	resolveNativeApprovalChoice,
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

test("grouped approval choices preserve exact, folder, persistence, and denial intent", () => {
	const exact = [
		{ kind: "write" as const, path: "/state/a", directory: false },
		{ kind: "write" as const, path: "/state/b", directory: false },
	];
	const folder = { kind: "write" as const, path: "/state", directory: true };
	const request = { permissions: exact, folderAlternative: folder };
	assert.deepEqual(resolveNativeApprovalChoice(request, "exact-once"), {
		permissions: exact,
		persistent: false,
	});
	assert.deepEqual(resolveNativeApprovalChoice(request, "exact-always"), {
		permissions: exact,
		persistent: true,
	});
	assert.deepEqual(resolveNativeApprovalChoice(request, "folder-once"), {
		permissions: [folder],
		persistent: false,
	});
	assert.deepEqual(resolveNativeApprovalChoice(request, "folder-always"), {
		permissions: [folder],
		persistent: true,
	});
	assert.equal(resolveNativeApprovalChoice(request, "deny"), undefined);
	assert.equal(
		resolveNativeApprovalChoice({ permissions: exact }, "folder-once"),
		undefined,
	);
});

test("model history keeps only the final internal permission attempt", () => {
	assert.equal(
		modelVisibleApprovedRetryOutput(
			"first failure\n" +
				"\n[Retrying command with approved IO rights]\n" +
				"second failure\n",
		),
		"\n[Retrying command with approved IO rights]\nsecond failure\n",
	);
	assert.equal(
		modelVisibleApprovedRetryOutput("failure without an approved retry\n"),
		"failure without an approved retry\n",
	);
});

test("native structured denial prompts and retries without parsing app output", async () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-native-retry-"));
	const path = `/home/sandbox-user/pi-native-retry-${process.pid}/state.db`;
	const expectedPath = canonicalize(path);
	const broker = new FakeBroker((request, onData) => {
		if (request.id.endsWith("attempt-0")) {
			onData(Buffer.from("service unavailable\n"));
			return failed(path);
		}
		onData(Buffer.from("created\n"));
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
		async approve(request) {
			assert.equal(Buffer.concat(output).toString("utf8"), "service unavailable\n");
			approvals.push(...request.permissions.map((permission) => permission.path));
			return request.permissions;
		},
	});

	const result = await operations.exec("issues search", cwd, {
		onData: (data) => output.push(data),
	});
	assert.equal(result.exitCode, 0);
	assert.deepEqual(approvals, [expectedPath]);
	assert.deepEqual(
		broker.requests.map((request) => request.id),
		["tool-1/attempt-0", "tool-1/attempt-1"],
	);
	assert.equal(broker.requests[0]?.policy.grants.length, 0);
	assert.deepEqual(broker.requests[1]?.policy.grants, [
		{
			access: "write",
			path: expectedPath,
			scope: "file",
			missing_path: "create_file",
		},
	]);
	assert.equal(
		Buffer.concat(output).toString("utf8"),
		"service unavailable\n" +
			"\n[Retrying command with approved IO rights]\n" +
			"created\n" +
			"[Command completed successfully after approved IO retry]\n",
	);
	assert.equal(
		modelVisibleApprovedRetryOutput(Buffer.concat(output).toString("utf8")),
		"\n[Retrying command with approved IO rights]\n" +
			"created\n" +
			"[Command completed successfully after approved IO retry]\n",
	);
});

test("native denied retry keeps the failed attempt output", async () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-native-denied-retry-"));
	const path = `/home/sandbox-user/pi-native-denied-retry-${process.pid}/state.db`;
	const broker = new FakeBroker((_request, onData) => {
		onData(Buffer.from("permission denied\n"));
		return failed(path);
	});
	const output: Buffer[] = [];
	const operations = createApprovingNativeSandboxOps({
		client: broker,
		config: DEFAULT_CONFIG,
		initialPermissions: [],
		toolCallId: "tool-denied-retry",
		blockedPaths: [],
		async approve() {
			return undefined;
		},
	});

	const result = await operations.exec("issues search", cwd, {
		onData: (data) => output.push(data),
	});
	assert.equal(result.exitCode, 1);
	assert.equal(Buffer.concat(output).toString("utf8"), "permission denied\n");
	assert.equal(broker.requests.length, 1);
});

test("native Linux falls back to an exact stderr path when denial hints are unavailable", async () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-native-stderr-retry-"));
	const path = `/home/sandbox-user/pi-native-stderr-retry-${process.pid}`;
	const expectedPath = canonicalize(path);
	const broker = new FakeBroker((request, onData) => {
		if (request.id.endsWith("attempt-0")) {
			onData(
				Buffer.from(
					`mkdir: cannot create directory ‘${path}’: Read-only file system\n`,
				),
			);
			return { exitCode: 1, denials: [], denialsComplete: false };
		}
		onData(Buffer.from("initialized\n"));
		return { exitCode: 0, denials: [], denialsComplete: false };
	});
	const approvals: NativeApprovalRequest[] = [];
	const output: Buffer[] = [];
	const operations = createApprovingNativeSandboxOps({
		client: broker,
		config: DEFAULT_CONFIG,
		initialPermissions: [],
		toolCallId: "tool-stderr",
		blockedPaths: [],
		async approve(request) {
			approvals.push(request);
			return request.permissions;
		},
	});

	const result = await operations.exec("mkdir -p external", cwd, {
		onData(data) {
			output.push(data);
		},
	});

	assert.equal(result.exitCode, 0);
	assert.deepEqual(approvals[0]?.permissions, [
		{ kind: "write", path: expectedPath, directory: true },
	]);
	assert.equal(
		modelVisibleApprovedRetryOutput(Buffer.concat(output).toString("utf8")),
		"\n[Retrying command with approved IO rights]\n" +
			"initialized\n" +
			"[Command completed successfully after approved IO retry]\n",
	);
});

test("four sibling denials offer one recursive folder approval", async () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-native-group-"));
	const folder = fileURLToPath(new URL(".", import.meta.url));
	const paths = ["issues.sqlite", "issues.sqlite-wal", "issues.sqlite-shm", "issues.sqlite.lock"].map(
		(name) => join(folder, name),
	);
	const broker = new FakeBroker((request) =>
		request.id.endsWith("attempt-0")
			? {
					exitCode: 1,
					denials: paths.map((path) => ({
						operation: "file-write-create",
						path,
						process: "issues",
					})),
					denialsComplete: false,
				}
			: { exitCode: 0, denials: [], denialsComplete: false },
	);
	const approvals: NativeApprovalRequest[] = [];
	const operations = createApprovingNativeSandboxOps({
		client: broker,
		config: DEFAULT_CONFIG,
		initialPermissions: [],
		toolCallId: "tool-group",
		blockedPaths: [],
		async approve(request) {
			approvals.push(request);
			return request.folderAlternative ? [request.folderAlternative] : request.permissions;
		},
	});

	const result = await operations.exec("issues search", cwd, { onData() {} });
	assert.equal(result.exitCode, 0);
	assert.equal(approvals.length, 1);
	assert.deepEqual(
		approvals[0]?.permissions.map((permission) => permission.path),
		paths,
	);
	assert.deepEqual(approvals[0]?.folderAlternative, {
		kind: "write",
		path: canonicalize(folder),
		directory: true,
	});
	assert.deepEqual(broker.requests[1]?.policy.grants, [
		{
			access: "write",
			path: canonicalize(folder),
			scope: "tree",
			missing_path: "reject",
		},
	]);
});

test("grouped sibling denials can retain exact file rights", async () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-native-group-exact-"));
	const folder = fileURLToPath(new URL(".", import.meta.url));
	const paths = ["exact.db", "exact.db-wal", "exact.db-shm", "exact.db-lock"].map((name) =>
		join(folder, name),
	);
	const broker = new FakeBroker((request) =>
		request.id.endsWith("attempt-0")
			? {
					exitCode: 1,
					denials: paths.map((path) => ({
						operation: "file-write-create",
						path,
						process: "issues",
					})),
					denialsComplete: false,
				}
			: { exitCode: 0, denials: [], denialsComplete: false },
	);
	const operations = createApprovingNativeSandboxOps({
		client: broker,
		config: DEFAULT_CONFIG,
		initialPermissions: [],
		toolCallId: "tool-group-exact",
		blockedPaths: [],
		async approve(request) {
			return request.permissions;
		},
	});

	const result = await operations.exec("issues search", cwd, { onData() {} });
	assert.equal(result.exitCode, 0);
	assert.deepEqual(
		broker.requests[1]?.policy.grants.map((permission) => ({
			path: permission.path,
			scope: permission.scope,
		})),
		paths.map((path) => ({ path, scope: "file" })),
	);
});

test("sibling denials accumulate across retries before offering the folder", async () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-native-group-history-"));
	const folder = fileURLToPath(new URL(".", import.meta.url));
	const paths = ["state.db", "state.db-wal", "state.db-shm", "state.db-lock"].map((name) =>
		join(folder, name),
	);
	const broker = new FakeBroker((request) => {
		const attempt = Number(request.id.match(/attempt-(\d+)$/)?.[1] ?? 0);
		if (attempt >= paths.length) return { exitCode: 0, denials: [], denialsComplete: false };
		return failed(paths[attempt] ?? paths[0]!);
	});
	const approvalSizes: number[] = [];
	const operations = createApprovingNativeSandboxOps({
		client: broker,
		config: DEFAULT_CONFIG,
		initialPermissions: [],
		toolCallId: "tool-history",
		blockedPaths: [],
		async approve(request) {
			approvalSizes.push(request.permissions.length);
			return request.folderAlternative ? [request.folderAlternative] : request.permissions;
		},
	});

	const result = await operations.exec("issues search", cwd, { onData() {} });
	assert.equal(result.exitCode, 0);
	assert.deepEqual(approvalSizes, [1, 1, 1, 4]);
	assert.equal(broker.requests.length, 5);
	assert.equal(
		broker.requests[4]?.policy.grants.some(
			(permission) => permission.path === canonicalize(folder) && permission.scope === "tree",
		),
		true,
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
		async approve(request) {
			prompts += 1;
			return request.permissions;
		},
	});

	const result = await operations.exec("false", cwd, { onData() {} });
	assert.equal(result.exitCode, 1);
	assert.equal(prompts, 0);
	assert.equal(broker.requests.length, 1);
});

test("cancellation during native approval prevents retry", async () => {
	const cwd = mkdtempSync(join(tmpdir(), "pi-native-abort-"));
	const path = `/home/sandbox-user/pi-native-abort-${process.pid}/state.db`;
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
		approve(_request, signal) {
			approvalStarted?.();
			return new Promise<undefined>((resolve) => {
				if (signal?.aborted) resolve(undefined);
				else signal?.addEventListener("abort", () => resolve(undefined), { once: true });
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
