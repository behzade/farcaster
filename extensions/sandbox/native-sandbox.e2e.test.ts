import assert from "node:assert/strict";
import {
	accessSync,
	constants,
	existsSync,
	mkdtempSync,
	mkdirSync,
	readFileSync,
	rmSync,
	writeFileSync,
} from "node:fs";
import { createServer, type Server } from "node:http";
import { homedir, tmpdir } from "node:os";
import { delimiter, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { after, before, test } from "node:test";
import type { BashOperations } from "@earendil-works/pi-coding-agent";
import { NativeBackgroundJobs } from "./native-background-jobs.ts";
import { SandboxBrokerClient } from "./broker-client.ts";
import { createNativeSandboxOps } from "./native-sandbox-ops.ts";
import type { NativeFilePermission } from "./broker-policy.ts";
import {
	developmentCacheRoot,
	ensureDevelopmentCacheDirectories,
} from "./development-caches.ts";
import { DEFAULT_CONFIG } from "./sandbox-config.ts";

const defaultBrokerPath = fileURLToPath(
	new URL("../../sandbox-broker/target/debug/pi-sandbox-broker", import.meta.url),
);
const brokerPath = process.env.PI_SANDBOX_BROKER_E2E ?? defaultBrokerPath;
const bunPath = findOnPath("bun");
if (!existsSync(brokerPath)) {
	throw new Error(
		"build the broker first: cargo build --manifest-path sandbox-broker/Cargo.toml",
	);
}
const skip = false;

let workspace = "";
let fixtureParent = "";
let fixture = "";
let client: SandboxBrokerClient;

before(async () => {
	workspace = mkdtempSync(join(tmpdir(), "pi-sandbox-e2e-workspace-"));
	fixtureParent = join(homedir(), ".pi-sandbox-e2e-files");
	mkdirSync(fixtureParent, { recursive: true });
	ensureDevelopmentCacheDirectories(DEFAULT_CONFIG.developmentCache);
	fixture = mkdtempSync(join(fixtureParent, "run-"));
	client = await SandboxBrokerClient.start(
		brokerPath,
		process.platform,
		developmentCacheRoot(DEFAULT_CONFIG.developmentCache),
	);
});

after(async () => {
	await client.shutdown();
	rmSync(workspace, { recursive: true, force: true });
	rmSync(fixtureParent, { recursive: true, force: true });
});

test("a denied command runs once and does not mutate the target", { skip }, async () => {
	const target = makeFixture("denied.txt");
	const attempts = join(workspace, "denied-attempts.txt");
	const ops = createNativeSandboxOps(client, DEFAULT_CONFIG, [], [], "e2e-denied-once");
	const result = await run(ops, `printf x >> ${quote(attempts)}; printf denied > ${quote(target)}`);

	assert.notEqual(result.exitCode, 0, result.output);
	assert.equal(readFileSync(target, "utf8"), "before");
	assert.equal(readFileSync(attempts, "utf8"), "x");
});

test("an active exact-file project grant reaches the real sandbox", { skip }, async () => {
	const target = makeFixture("single.txt");
	const grants: NativeFilePermission[] = [{ kind: "write", path: target, directory: false }];
	const result = await run(createNativeSandboxOps(client, DEFAULT_CONFIG, grants, [], "e2e-file-grant"), `printf single > ${quote(target)}`);

	assert.equal(result.exitCode, 0, result.output);
	assert.equal(readFileSync(target, "utf8"), "single");
});

test("an active tree project grant covers nested paths", { skip }, async () => {
	const target = makeFixture("deep/a path/with/many/levels/value.txt");
	const root = join(fixture, "deep");
	const grants: NativeFilePermission[] = [{ kind: "write", path: root, directory: true }];
	const result = await run(createNativeSandboxOps(client, DEFAULT_CONFIG, grants, [], "e2e-tree-grant"), `printf nested > ${quote(target)}`);

	assert.equal(result.exitCode, 0, result.output);
	assert.equal(readFileSync(target, "utf8"), "nested");
});

test(
	"Bun treats a protected optional env file as missing",
	{ skip: bunPath === undefined ? "set PI_BUN_E2E or put bun on PATH" : skip },
	async () => {
		assert.ok(bunPath);
		writeFileSync(
			join(workspace, "package.json"),
			readFileSync(new URL("../../apps/pi-terminal/package.json", import.meta.url)),
		);
		writeFileSync(
			join(workspace, "bun.lock"),
			readFileSync(new URL("../../apps/pi-terminal/bun.lock", import.meta.url)),
		);
		writeFileSync(join(workspace, ".env.local"), "PI_CONCEAL_E2E_SECRET=must-not-leak\n");
		const ops = createNativeSandboxOps(client, DEFAULT_CONFIG, [], [], "e2e-bun-conceal");
		const bun = quote(bunPath);
		const noSecret = quote("process.exit(process.env.PI_CONCEAL_E2E_SECRET ? 23 : 0)");
		const result = await run(ops, `${bun} -e ${noSecret} && ${bun} pm ls`);

		assert.equal(result.exitCode, 0, result.output);
		assert.doesNotMatch(result.output, /must-not-leak|PermissionDenied/);
	},
);

test("Seatbelt still blocks a protected read after dropping the conceal shim", { skip }, async () => {
	const secret = join(workspace, ".env.local");
	writeFileSync(secret, "PI_CONCEAL_BACKSTOP=must-not-leak\n");
	const ops = createNativeSandboxOps(client, DEFAULT_CONFIG, [], [], "e2e-conceal-backstop");
	const result = await run(
		ops,
		`/usr/bin/env -u DYLD_INSERT_LIBRARIES -u PI_SANDBOX_CONCEALED_PATHS /bin/cat ${quote(secret)}`,
	);

	assert.notEqual(result.exitCode, 0, result.output);
	assert.doesNotMatch(result.output, /must-not-leak/);
});

test("managed development caches permit public repository .env files", { skip }, async () => {
	const cacheFixture = join(
		developmentCacheRoot(DEFAULT_CONFIG.developmentCache),
		`e2e-public-env-${process.pid}`,
	);
	const publicEnv = join(cacheFixture, "checkout", ".env.toml");
	mkdirSync(dirname(publicEnv), { recursive: true });
	try {
		const ops = createNativeSandboxOps(client, DEFAULT_CONFIG, [], [], "e2e-cache-public-env");
		const result = await run(
			ops,
			`printf public-cache > ${quote(publicEnv)} && /bin/cat ${quote(publicEnv)}`,
		);
		assert.equal(result.exitCode, 0, result.output);
		assert.equal(result.output, "public-cache");
	} finally {
		rmSync(cacheFixture, { recursive: true, force: true });
	}
});

function findOnPath(name: string): string | undefined {
	const override = process.env[`PI_${name.toUpperCase()}_E2E`];
	for (const candidate of [
		...(override ? [override] : []),
		...(process.env.PATH ?? "").split(delimiter).map((directory) => join(directory, name)),
	]) {
		try {
			accessSync(candidate, constants.X_OK);
			return candidate;
		} catch {
			// Keep looking.
		}
	}
	return undefined;
}

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

test("an active network_local project right can bind and query an ephemeral port", { skip }, async () => {
	const ops = createNativeSandboxOps(
		client,
		DEFAULT_CONFIG,
		[],
		[],
		"e2e-network-local",
		true,
	);
	const command =
		`python3 -c ${quote(
			"import socket; s=socket.socket(); s.bind(('127.0.0.1',0)); s.listen(); " +
				"c=socket.create_connection(s.getsockname()); a,_=s.accept(); " +
				"c.sendall(b'local-ok'); print(a.recv(8).decode(),end='')",
		)}`;
	const result = await run(ops, command);
	assert.equal(result.exitCode, 0, result.output);
	assert.equal(result.output, "local-ok");
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
