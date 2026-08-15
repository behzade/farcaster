import assert from "node:assert/strict";
import {
	existsSync,
	mkdtempSync,
	mkdirSync,
	readFileSync,
	rmSync,
	writeFileSync,
} from "node:fs";
import { createServer, type Server } from "node:http";
import { homedir, tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { after, before, test } from "node:test";
import type { BashOperations } from "@earendil-works/pi-coding-agent";
import { NativeBackgroundJobs } from "./native-background-jobs.ts";
import { SandboxBrokerClient } from "./broker-client.ts";
import {
	createApprovingNativeSandboxOps,
	createNativeSandboxOps,
} from "./native-sandbox-ops.ts";
import type { NativeFilePermission } from "./native-denials.ts";
import { DEFAULT_CONFIG } from "./sandbox-config.ts";

const defaultBrokerPath = fileURLToPath(
	new URL("../../sandbox-broker/target/debug/pi-sandbox-broker", import.meta.url),
);
const brokerPath = process.env.PI_SANDBOX_BROKER_E2E ?? defaultBrokerPath;
if (!existsSync(brokerPath)) {
	throw new Error(
		"build the broker first: cargo build --manifest-path sandbox-broker/Cargo.toml",
	);
}
const skip = false;

let workspace = "";
let fixture = "";
let client: SandboxBrokerClient;

before(async () => {
	workspace = mkdtempSync(join(tmpdir(), "pi-sandbox-e2e-workspace-"));
	fixture = mkdtempSync(join(homedir(), ".pi-sandbox-e2e-files-"));
	client = await SandboxBrokerClient.start(brokerPath);
});

after(async () => {
	await client.shutdown();
	rmSync(workspace, { recursive: true, force: true });
	rmSync(fixture, { recursive: true, force: true });
});

test("single-file approval retries the real sandbox with the exact file", { skip }, async () => {
	const target = makeFixture("single.txt");
	const approvals: NativeFilePermission[][] = [];
	const ops = approvingOps("e2e-single", approvals);
	const result = await run(ops, `printf single > ${quote(target)}`);

	assert.equal(result.exitCode, 0, result.output);
	assert.equal(readFileSync(target, "utf8"), "single");
	assert.deepEqual(flattenPaths(approvals), [target]);
});

test("multi-file approval retains every exact path", { skip }, async () => {
	const targets = [makeFixture("multi/one.txt"), makeFixture("multi/two.txt")];
	const approvals: NativeFilePermission[][] = [];
	const ops = approvingOps("e2e-multi", approvals);
	const result = await run(
		ops,
		targets.map((path, index) => `printf value-${index} > ${quote(path)}`).join("; "),
	);

	assert.equal(result.exitCode, 0, result.output);
	assert.deepEqual(targets.map((path) => readFileSync(path, "utf8")), ["value-0", "value-1"]);
	assert.deepEqual(flattenPaths(approvals), targets);
});

test("grouped sibling denials offer a folder but exact approval stays exact", { skip }, async () => {
	const targets = Array.from({ length: 4 }, (_, index) => makeFixture(`siblings/file-${index}.txt`));
	const approvals: NativeFilePermission[][] = [];
	const folderAlternatives: NativeFilePermission[] = [];
	const ops = createApprovingNativeSandboxOps({
		client,
		config: DEFAULT_CONFIG,
		initialPermissions: [],
		toolCallId: "e2e-siblings",
		blockedPaths: [],
		approve: async (request) => {
			approvals.push([...request.permissions]);
			if (request.folderAlternative) folderAlternatives.push(request.folderAlternative);
			return request.permissions;
		},
	});
	const result = await run(
		ops,
		targets.map((path, index) => `printf sibling-${index} > ${quote(path)}`).join("; "),
	);

	assert.equal(result.exitCode, 0, result.output);
	assert.deepEqual(flattenPaths(approvals), targets);
	assert.ok(
		folderAlternatives.some(
			(permission) => permission.directory && permission.path === dirname(targets[0]),
		),
		"expected an exact sibling folder alternative",
	);
});

test("deep nested paths keep every slash and space through approval", { skip }, async () => {
	const target = makeFixture("deep/a path/with/many/levels/value.txt");
	const approvals: NativeFilePermission[][] = [];
	const ops = approvingOps("e2e-nested", approvals);
	const result = await run(ops, `printf nested > ${quote(target)}`);

	assert.equal(result.exitCode, 0, result.output);
	assert.equal(readFileSync(target, "utf8"), "nested");
	assert.deepEqual(flattenPaths(approvals), [target]);
});

test("one approved hostname reaches one port only through the proxy", { skip }, async () => {
	await withServers(1, async ([server]) => {
		const ops = createNativeSandboxOps(
			client,
			DEFAULT_CONFIG,
			[],
			["localhost"],
			"e2e-network-host",
		);
		const result = await run(ops, curl(`http://localhost:${server.port}/host`));
		assert.equal(result.exitCode, 0, result.output);
		assert.equal(result.output, "server-0:/host");
	});
});

test("several approved hosts and an IP grant work across several ports", { skip }, async () => {
	await withServers(2, async ([first, second]) => {
		const ops = createNativeSandboxOps(
			client,
			DEFAULT_CONFIG,
			[],
			["localhost", "127.0.0.1"],
			"e2e-network-many",
		);
		const command = [
			curl(`http://localhost:${first.port}/one`),
			curl(`http://127.0.0.1:${first.port}/two`),
			curl(`http://127.0.0.1:${second.port}/three`),
		].join("; printf '\\n'; ");
		const result = await run(ops, command);
		assert.equal(result.exitCode, 0, result.output);
		assert.deepEqual(result.output.split("\n"), [
			"server-0:/one",
			"server-0:/two",
			"server-1:/three",
		]);
	});
});

test("an unapproved host, direct bypass, and blocked network all fail", { skip }, async () => {
	await withServers(1, async ([server]) => {
		const wrongHost = createNativeSandboxOps(
			client,
			DEFAULT_CONFIG,
			[],
			["localhost"],
			"e2e-network-wrong-host",
		);
		const denied = await run(wrongHost, curl(`http://127.0.0.1:${server.port}/denied`));
		assert.notEqual(denied.exitCode, 0, denied.output);

		const approvedIp = createNativeSandboxOps(
			client,
			DEFAULT_CONFIG,
			[],
			["127.0.0.1"],
			"e2e-network-bypass",
		);
		const bypass = await run(
			approvedIp,
			`curl --noproxy '*' --fail --silent --show-error ${quote(`http://127.0.0.1:${server.port}/bypass`)}`,
		);
		assert.notEqual(bypass.exitCode, 0, bypass.output);

		const blocked = createNativeSandboxOps(
			client,
			DEFAULT_CONFIG,
			[],
			[],
			"e2e-network-blocked",
		);
		const noGrant = await run(blocked, curl(`http://127.0.0.1:${server.port}/blocked`));
		assert.notEqual(noGrant.exitCode, 0, noGrant.output);
	});
});

test("native background jobs accept input, retain output, stop, and clean up", { skip }, async () => {
	const jobs = new NativeBackgroundJobs(brokerPath);
	try {
		assert.equal(
			await jobs.start({
				name: "e2e-job",
				command: "IFS= read -r line; printf 'received:%s\\n' \"$line\"; sleep 30",
				cwd: workspace,
				config: DEFAULT_CONFIG,
				permissions: [],
				networkHosts: [],
			}),
			"started e2e-job",
		);
		assert.match(jobs.status("e2e-job"), /state=running/);
		assert.equal(jobs.write("e2e-job", Buffer.from("hello\n")), "sent input to e2e-job");
		await waitFor(() => jobs.read("e2e-job", 20).includes("received:hello"));
		assert.match(jobs.read("e2e-job", 20), /received:hello/);
		assert.equal(await jobs.stop("e2e-job"), "stopped e2e-job");
		assert.equal(jobs.list(), "no background jobs");
	} finally {
		await jobs.shutdown();
	}
});

function approvingOps(id: string, approvals: NativeFilePermission[][]): BashOperations {
	return createApprovingNativeSandboxOps({
		client,
		config: DEFAULT_CONFIG,
		initialPermissions: [],
		toolCallId: id,
		blockedPaths: [],
		approve: async (request) => {
			approvals.push([...request.permissions]);
			return request.permissions;
		},
	});
}

async function run(
	ops: BashOperations,
	command: string,
): Promise<{ exitCode: number | null; output: string }> {
	const output: Buffer[] = [];
	const result = await ops.exec(command, workspace, {
		onData: (data) => output.push(data),
	});
	return { exitCode: result.exitCode, output: Buffer.concat(output).toString("utf8") };
}

function makeFixture(relative: string): string {
	const path = join(fixture, relative);
	mkdirSync(dirname(path), { recursive: true });
	writeFileSync(path, "before");
	return path;
}

function flattenPaths(approvals: readonly NativeFilePermission[][]): string[] {
	return [...new Set(approvals.flat().map((permission) => permission.path))].sort();
}

function quote(value: string): string {
	return `'${value.replaceAll("'", `'\"'\"'`)}'`;
}

function curl(url: string): string {
	return `curl --fail --silent --show-error ${quote(url)}`;
}

interface TestServer {
	server: Server;
	port: number;
}

async function withServers(
	count: number,
	body: (servers: TestServer[]) => Promise<void>,
): Promise<void> {
	const servers = await Promise.all(
		Array.from({ length: count }, (_, index) => startServer(index)),
	);
	try {
		await body(servers);
	} finally {
		await Promise.all(servers.map(({ server }) => closeServer(server)));
	}
}

function startServer(index: number): Promise<TestServer> {
	return new Promise((resolve, reject) => {
		const server = createServer((request, response) => {
			response.end(`server-${index}:${request.url}`);
		});
		server.once("error", reject);
		server.listen(0, "127.0.0.1", () => {
			server.removeListener("error", reject);
			const address = server.address();
			if (!address || typeof address === "string") {
				reject(new Error("test server has no TCP port"));
				return;
			}
			resolve({ server, port: address.port });
		});
	});
}

function closeServer(server: Server): Promise<void> {
	return new Promise((resolve, reject) =>
		server.close((error) => (error ? reject(error) : resolve())),
	);
}

async function waitFor(check: () => boolean): Promise<void> {
	const deadline = Date.now() + 5_000;
	while (!check()) {
		if (Date.now() >= deadline) throw new Error("timed out waiting for background output");
		await new Promise((resolve) => setTimeout(resolve, 25));
	}
}
