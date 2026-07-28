import assert from "node:assert/strict";
import test, { type TestContext } from "node:test";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import type { Tool as McpTool } from "@modelcontextprotocol/sdk/types.js";
import {
	CallToolRequestSchema,
	ListToolsRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";
import { MCP_APPROVAL_SERVICE_KEY } from "./approval-service.ts";
import {
	closeConnection,
	piToolName,
	redirectSafeFetch,
	registerMcpAdapter,
} from "./index.ts";

type RegisteredTool = {
	name: string;
	execute: (...args: any[]) => Promise<any>;
};
type Page = { tools: McpTool[]; nextCursor?: string };
type ConnectionConfig = {
	page(cursor: string | undefined): Page;
};

const endpointInput = {
	action: "enable",
	name: "demo",
	host: "127.0.0.1",
	port: 3845,
	path: "/mcp",
} as const;
const endpointUrl = "http://127.0.0.1:3845/mcp";

function tool(name: string): McpTool {
	return { name, description: `${name} tool`, inputSchema: { type: "object" } };
}

function fakePi(failRegistration?: string) {
	const tools = new Map<string, RegisteredTool>();
	let active: string[] = [];
	let shutdown: (() => Promise<void>) | undefined;
	let failed = false;
	const pi = {
		registerTool(registered: RegisteredTool) {
			if (registered.name === failRegistration && !failed) {
				failed = true;
				throw new Error(`registration failed: ${registered.name}`);
			}
			tools.set(registered.name, registered);
			if (!active.includes(registered.name)) active.push(registered.name);
		},
		getActiveTools() {
			return [...active];
		},
		getAllTools() {
			return [...tools.values()];
		},
		setActiveTools(names: string[]) {
			active = [...names];
		},
		on(event: string, handler: () => Promise<void>) {
			if (event === "session_shutdown") shutdown = handler;
		},
	};
	return {
		pi,
		tools,
		active: () => [...active],
		shutdown: async () => shutdown?.(),
	};
}

function installApproval(t: TestContext, approved = new Set([endpointUrl])): void {
	const registry = globalThis as Record<symbol, unknown>;
	registry[MCP_APPROVAL_SERVICE_KEY] = {
		version: 1,
		owner: {},
		isEndpointApproved: (endpoint: string) => approved.has(endpoint),
	};
	t.after(() => delete registry[MCP_APPROVAL_SERVICE_KEY]);
}

function connectionFactory(
	configs: ConnectionConfig[],
	servers: Server[],
	closed: { count: number; terminated: number },
) {
	let connectionIndex = 0;
	return async () => {
		const config = configs[Math.min(connectionIndex, configs.length - 1)];
		connectionIndex += 1;
		const server = new Server(
			{ name: "test-server", version: "1.0.0" },
			{ capabilities: { tools: {} } },
		);
		server.setRequestHandler(ListToolsRequestSchema, async (request) =>
			config.page(request.params?.cursor),
		);
		server.setRequestHandler(CallToolRequestSchema, async (request) => {
			if (request.params.name === "fail") {
				return { isError: true, content: [{ type: "text", text: "upstream failed" }] };
			}
			if (request.params.name === "image") {
				return { content: [{ type: "image", data: "aGVsbG8=", mimeType: "image/png" }] };
			}
			return {
				content: [{ type: "text", text: String(request.params.arguments?.text ?? "") }],
			};
		});
		const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
		(clientTransport as InMemoryTransport & { terminateSession: () => Promise<void> }).terminateSession = async () => {
			closed.terminated += 1;
		};
		await server.connect(serverTransport);
		servers.push(server);
		const client = new Client({ name: "test-client", version: "1.0.0" });
		const close = client.close.bind(client);
		client.close = async () => {
			closed.count += 1;
			await close();
		};
		return { client, transport: clientTransport };
	};
}

function setupAdapter(t: TestContext, configs: ConnectionConfig[], failRegistration?: string) {
	installApproval(t);
	const servers: Server[] = [];
	const closed = { count: 0, terminated: 0 };
	t.after(async () => Promise.all(servers.map((server) => server.close().catch(() => undefined))));
	const runtime = fakePi(failRegistration);
	registerMcpAdapter(runtime.pi as never, connectionFactory(configs, servers, closed));
	return { runtime, closed };
}

test("one control tool discovers every page and returns native text and image content", async (t) => {
	const longUpstreamName = `very_${"long_".repeat(20)}tool`;
	const { runtime, closed } = setupAdapter(t, [
		{
			page: (cursor) =>
				cursor === undefined
					? { tools: [tool("echo"), tool("fail")], nextCursor: "page-2" }
					: { tools: [tool("image"), tool(longUpstreamName)] },
		},
	]);

	assert.deepEqual(runtime.active(), ["mcp"]);
	const control = runtime.tools.get("mcp");
	assert.ok(control);
	await control.execute("enable-call", endpointInput, undefined);
	assert.ok(runtime.active().includes("mcp_demo_echo"));
	assert.ok(runtime.active().includes("mcp_demo_image"));

	const echo = runtime.tools.get("mcp_demo_echo");
	assert.ok(echo);
	const echoed = await echo.execute("echo-call", { text: "hello" }, undefined);
	assert.deepEqual(echoed.content, [{ type: "text", text: "hello" }]);

	const image = runtime.tools.get("mcp_demo_image");
	assert.ok(image);
	const imageResult = await image.execute("image-call", {}, undefined);
	assert.deepEqual(imageResult.content, [
		{ type: "image", data: "aGVsbG8=", mimeType: "image/png" },
	]);

	const longName = piToolName("demo", longUpstreamName);
	assert.equal(longName.length, 64);
	assert.equal(longName, piToolName("demo", longUpstreamName));
	assert.ok(runtime.active().includes(longName));

	const fail = runtime.tools.get("mcp_demo_fail");
	assert.ok(fail);
	await assert.rejects(() => fail.execute("fail-call", {}, undefined), /upstream failed/);
	await runtime.shutdown();
	assert.equal(closed.terminated, 1);
	assert.equal(closed.count, 1);
});

test("disable then re-enable overwrites and reactivates adapter-owned historical tools", async (t) => {
	const pages = [{ page: () => ({ tools: [tool("echo")] }) }];
	const { runtime } = setupAdapter(t, pages);
	const control = runtime.tools.get("mcp");
	assert.ok(control);
	await control.execute("enable-1", endpointInput);
	await control.execute("disable", { action: "disable", name: "demo" });
	assert.deepEqual(runtime.active(), ["mcp"]);
	await control.execute("enable-2", endpointInput);
	assert.ok(runtime.active().includes("mcp_demo_echo"));
	const echo = runtime.tools.get("mcp_demo_echo");
	assert.ok(echo);
	assert.deepEqual((await echo.execute("echo", { text: "again" })).content, [
		{ type: "text", text: "again" },
	]);
});

test("partial registration failure deactivates every planned name and closes the new client", async (t) => {
	const { runtime, closed } = setupAdapter(
		t,
		[{ page: () => ({ tools: [tool("alpha"), tool("beta")] }) }],
		"mcp_demo_beta",
	);
	const control = runtime.tools.get("mcp");
	assert.ok(control);
	await assert.rejects(() => control.execute("enable", endpointInput), /registration failed/);
	assert.deepEqual(runtime.active(), ["mcp"]);
	assert.equal(closed.count, 1);
	const alpha = runtime.tools.get("mcp_demo_alpha");
	assert.ok(alpha);
	await assert.rejects(() => alpha.execute("stale", {}), /disabled or was replaced/);
});

test("repeated tools/list cursors fail closed", async (t) => {
	const { runtime, closed } = setupAdapter(t, [
		{ page: () => ({ tools: [tool("echo")], nextCursor: "repeat" }) },
	]);
	const control = runtime.tools.get("mcp");
	assert.ok(control);
	await assert.rejects(() => control.execute("enable", endpointInput), /repeated cursor/);
	assert.deepEqual(runtime.active(), ["mcp"]);
	assert.equal(closed.count, 1);
});

test("discovery bounds unique pages and total tools", async (t) => {
	let pageNumber = 0;
	const pageLimited = setupAdapter(t, [
		{
			page: () => ({ tools: [], nextCursor: `page-${++pageNumber}` }),
		},
	]);
	const pageControl = pageLimited.runtime.tools.get("mcp");
	assert.ok(pageControl);
	await assert.rejects(() => pageControl.execute("enable-pages", endpointInput), /exceeded 32 pages/);

	const toolLimited = setupAdapter(t, [
		{
			page: () => ({
				tools: Array.from({ length: 129 }, (_, index) => tool(`tool_${index}`)),
			}),
		},
	]);
	const toolControl = toolLimited.runtime.tools.get("mcp");
	assert.ok(toolControl);
	await assert.rejects(() => toolControl.execute("enable-tools", endpointInput), /more than 128 tools/);
});

test("connection close proceeds when session termination does not settle", async () => {
	let closed = false;
	await closeConnection(
		{
			client: {
				close: async () => {
					closed = true;
				},
			},
			transport: {
				terminateSession: () => new Promise(() => undefined),
			},
		} as never,
		5,
	);
	assert.equal(closed, true);
});

test("missing approval service blocks before connection creation", async () => {
	delete (globalThis as Record<symbol, unknown>)[MCP_APPROVAL_SERVICE_KEY];
	const runtime = fakePi();
	let connections = 0;
	registerMcpAdapter(runtime.pi as never, () => {
		connections += 1;
		throw new Error("must not connect");
	});
	const control = runtime.tools.get("mcp");
	assert.ok(control);
	await assert.rejects(() => control.execute("enable", endpointInput), /approval service is unavailable/);
	assert.equal(connections, 0);
});

test("redirect-safe fetch overrides caller redirect policy", async () => {
	let observed: RequestInit | undefined;
	const wrapped = redirectSafeFetch(async (_url, init) => {
		observed = init;
		return new Response("ok");
	});
	await wrapped("https://example.com/mcp", { redirect: "follow", method: "GET" });
	assert.equal(observed?.redirect, "error");
	assert.equal(observed?.method, "GET");
});
