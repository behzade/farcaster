import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import type { FetchLike, Transport } from "@modelcontextprotocol/sdk/shared/transport.js";
import type { Tool as McpTool } from "@modelcontextprotocol/sdk/types.js";
import { StringEnum } from "@earendil-works/pi-ai";
import {
	DEFAULT_MAX_BYTES,
	DEFAULT_MAX_LINES,
	formatSize,
	truncateHead,
	type ExtensionAPI,
} from "@earendil-works/pi-coding-agent";
import { createHash } from "node:crypto";
import { Type } from "typebox";
import { requireMcpEndpointApproval } from "./approval-service.ts";
import { normalizeMcpEndpoint, type McpEndpoint } from "./endpoint.ts";

const MAX_IMAGE_BYTES = 10 * 1024 * 1024;
const MAX_PI_TOOL_NAME_LENGTH = 64;
const MAX_TOOL_LIST_PAGES = 32;
const MAX_DISCOVERED_TOOLS = 128;
const SESSION_TERMINATION_TIMEOUT_MS = 2_000;
const SERVER_NAME_PATTERN = /^[a-z][a-z0-9_-]{0,31}$/;

type Connection = { client: Client; transport: Transport };
type ConnectionFactory = (
	endpoint: McpEndpoint,
) => Promise<Connection> | Connection;
type ServerState = Connection & {
	endpoint: McpEndpoint;
	toolNames: Set<string>;
};
type McpContentItem = {
	type?: string;
	text?: string;
	mimeType?: string;
	data?: string;
};
type PiContent =
	| { type: "text"; text: string }
	| { type: "image"; data: string; mimeType: string };

function normalizeServerName(input: string): string {
	const name = input.trim().toLowerCase();
	if (!SERVER_NAME_PATTERN.test(name)) {
		throw new Error("MCP server name must start with a letter and contain at most 32 lowercase letters, numbers, underscores, or hyphens");
	}
	return name;
}

function upstreamToolSlug(input: string): string {
	const slug = input.toLowerCase().replace(/[^a-z0-9_-]+/g, "_").replace(/^_+|_+$/g, "");
	if (!slug) throw new Error(`MCP tool name cannot be normalized: ${JSON.stringify(input)}`);
	return slug;
}

export function piToolName(serverName: string, upstreamName: string): string {
	const prefix = `mcp_${serverName}_`;
	const slug = upstreamToolSlug(upstreamName);
	const fullName = `${prefix}${slug}`;
	if (fullName.length <= MAX_PI_TOOL_NAME_LENGTH) return fullName;
	const hash = createHash("sha256")
		.update(`${serverName}\0${upstreamName}`)
		.digest("hex")
		.slice(0, 8);
	const slugLength = MAX_PI_TOOL_NAME_LENGTH - prefix.length - hash.length - 1;
	return `${prefix}${slug.slice(0, slugLength)}_${hash}`;
}

function boundedText(input: string): string {
	const result = truncateHead(input, {
		maxBytes: DEFAULT_MAX_BYTES,
		maxLines: DEFAULT_MAX_LINES,
	});
	if (!result.truncated) return result.content;
	return `${result.content}\n\n[MCP output truncated: ${result.outputLines} of ${result.totalLines} lines (${formatSize(result.outputBytes)} of ${formatSize(result.totalBytes)}).]`;
}

function resultContent(result: Awaited<ReturnType<Client["callTool"]>>): PiContent[] {
	const items = Array.isArray(result.content) ? (result.content as McpContentItem[]) : [];
	const text: string[] = [];
	const images: Array<Extract<PiContent, { type: "image" }>> = [];
	let imageBytes = 0;

	for (const item of items) {
		if (item.type === "text" && typeof item.text === "string") {
			text.push(item.text);
			continue;
		}
		if (
			item.type === "image" &&
			typeof item.data === "string" &&
			typeof item.mimeType === "string"
		) {
			imageBytes += Buffer.byteLength(item.data, "base64");
			if (imageBytes <= MAX_IMAGE_BYTES) {
				images.push({ type: "image", data: item.data, mimeType: item.mimeType });
			} else {
				text.push(`[MCP image omitted after ${formatSize(MAX_IMAGE_BYTES)} cumulative image data.]`);
			}
			continue;
		}
		text.push(JSON.stringify(item));
	}

	if (items.length === 0 && result.structuredContent !== undefined) {
		text.push(JSON.stringify(result.structuredContent, null, 2));
	}
	const normalizedText = text.length > 0 ? boundedText(text.join("\n")) : undefined;
	const content: PiContent[] = [
		...(normalizedText ? [{ type: "text" as const, text: normalizedText }] : []),
		...images,
	];
	return content.length > 0
		? content
		: [{ type: "text", text: "MCP tool completed without output." }];
}

export function redirectSafeFetch(fetchImplementation: FetchLike): FetchLike {
	return (url, init) => fetchImplementation(url, { ...init, redirect: "error" });
}

function streamableHttpConnection(endpoint: McpEndpoint): Connection {
	return {
		client: new Client({ name: "pi-mcp-adapter", version: "0.2.0" }),
		transport: new StreamableHTTPClientTransport(new URL(endpoint.url), {
			requestInit: { redirect: "error" },
			fetch: redirectSafeFetch(globalThis.fetch.bind(globalThis)),
		}),
	};
}

async function listAllTools(client: Client, signal?: AbortSignal): Promise<McpTool[]> {
	const tools: McpTool[] = [];
	const seenCursors = new Set<string>();
	let cursor: string | undefined;
	let pageCount = 0;
	do {
		pageCount += 1;
		if (pageCount > MAX_TOOL_LIST_PAGES) {
			throw new Error(`MCP tools/list exceeded ${MAX_TOOL_LIST_PAGES} pages`);
		}
		const page = await client.listTools(cursor ? { cursor } : undefined, { signal });
		if (tools.length + page.tools.length > MAX_DISCOVERED_TOOLS) {
			throw new Error(`MCP server exposes more than ${MAX_DISCOVERED_TOOLS} tools`);
		}
		tools.push(...page.tools);
		cursor = page.nextCursor;
		if (cursor) {
			if (seenCursors.has(cursor)) {
				throw new Error(`MCP tools/list repeated cursor: ${cursor}`);
			}
			seenCursors.add(cursor);
		}
	} while (cursor);
	return tools;
}

async function settleOrTimeout(promise: Promise<unknown>, timeoutMs: number): Promise<void> {
	let timeout: ReturnType<typeof setTimeout> | undefined;
	try {
		await Promise.race([
			promise.catch(() => undefined),
			new Promise<void>((resolve) => {
				timeout = setTimeout(resolve, timeoutMs);
			}),
		]);
	} finally {
		if (timeout) clearTimeout(timeout);
	}
}

export async function closeConnection(
	connection: Connection,
	terminationTimeoutMs = SESSION_TERMINATION_TIMEOUT_MS,
): Promise<void> {
	const terminable = connection.transport as Transport & { terminateSession?: () => Promise<void> };
	if (typeof terminable.terminateSession === "function") {
		await settleOrTimeout(terminable.terminateSession(), terminationTimeoutMs);
	}
	await connection.client.close().catch(() => undefined);
}

export function registerMcpAdapter(
	pi: ExtensionAPI,
	connectionFactory: ConnectionFactory = streamableHttpConnection,
) {
	const servers = new Map<string, ServerState>();
	const adapterOwnedToolNames = new Set<string>();

	function deactivate(toolNames: ReadonlySet<string>): void {
		pi.setActiveTools(pi.getActiveTools().filter((toolName) => !toolNames.has(toolName)));
	}

	async function disconnect(name: string): Promise<number> {
		const current = servers.get(name);
		if (!current) return 0;
		deactivate(current.toolNames);
		servers.delete(name);
		await closeConnection(current);
		return current.toolNames.size;
	}

	function registerMcpTool(serverName: string, endpoint: McpEndpoint, tool: McpTool): string {
		const name = piToolName(serverName, tool.name);
		pi.registerTool({
			name,
			label: `${serverName}: ${tool.title ?? tool.name}`,
			description: `[${serverName} MCP at ${endpoint.url}] ${tool.description ?? tool.name}`,
			parameters: Type.Unsafe(tool.inputSchema),
			async execute(_toolCallId, params, signal) {
				const current = servers.get(serverName);
				if (!current || current.endpoint.url !== endpoint.url || !current.toolNames.has(name)) {
					throw new Error(`${serverName} MCP is disabled or was replaced; enable it again before using this tool.`);
				}
				const result = await current.client.callTool(
					{ name: tool.name, arguments: params as Record<string, unknown> },
					undefined,
					{ signal },
				);
				if (result.isError) {
					const message = resultContent(result)
						.filter((item): item is Extract<PiContent, { type: "text" }> => item.type === "text")
						.map((item) => item.text)
						.join("\n");
					throw new Error(message || `${serverName} MCP tool failed: ${tool.name}`);
				}
				return {
					content: resultContent(result),
					details: { server: serverName, endpoint: endpoint.url, upstreamTool: tool.name },
				};
			},
		});
		adapterOwnedToolNames.add(name);
		return name;
	}

	pi.registerTool({
		name: "mcp",
		label: "MCP server",
		description:
			"Enable or disable a Streamable HTTP MCP server. Enabling requires a short server name and one exact protocol, host, port, and path; it discovers and activates that server's tools for this session.",
		promptSnippet: "Lazily enable an approved Streamable HTTP MCP server when its tools are needed",
		executionMode: "sequential",
		parameters: Type.Object({
			action: StringEnum(["enable", "disable"] as const),
			name: Type.String({ description: "Short lowercase namespace for the server's discovered tools" }),
			protocol: Type.Optional(StringEnum(["http", "https"] as const)),
			host: Type.Optional(Type.String({ description: "Exact hostname or IP without scheme, port, or path" })),
			port: Type.Optional(Type.Integer({ minimum: 1, maximum: 65_535 })),
			path: Type.Optional(Type.String({ description: "Absolute MCP HTTP path, such as /mcp" })),
		}),
		async execute(_toolCallId, params, signal) {
			const name = normalizeServerName(params.name);
			if (params.action === "disable") {
				const count = await disconnect(name);
				return {
					content: [{ type: "text" as const, text: `Disabled ${name} MCP and deactivated ${count} tool(s).` }],
					details: { action: "disable", server: name, toolsRemoved: count },
				};
			}
			if (params.host === undefined || params.port === undefined || params.path === undefined) {
				throw new Error("Enabling MCP requires host, port, and path");
			}
			const endpoint = normalizeMcpEndpoint({
				protocol: params.protocol,
				host: params.host,
				port: params.port,
				path: params.path,
			});
			requireMcpEndpointApproval(endpoint.url);
			const connection = await connectionFactory(endpoint);
			let adopted = false;
			let plannedNames = new Set<string>();

			try {
				await connection.client.connect(connection.transport, { signal });
				const tools = await listAllTools(connection.client, signal);
				const names = tools.map((tool) => piToolName(name, tool.name));
				plannedNames = new Set(names);
				if (plannedNames.size !== names.length) {
					throw new Error(`${name} MCP exposes tool names that collide after normalization`);
				}
				const existing = new Set(pi.getAllTools().map((tool) => tool.name));
				const conflict = names.find(
					(toolName) => existing.has(toolName) && !adapterOwnedToolNames.has(toolName),
				);
				if (conflict) throw new Error(`MCP tool name conflicts with an existing Pi tool: ${conflict}`);

				await disconnect(name);
				const state: ServerState = { ...connection, endpoint, toolNames: plannedNames };
				servers.set(name, state);
				adopted = true;
				for (const tool of tools) registerMcpTool(name, endpoint, tool);
				pi.setActiveTools([...new Set([...pi.getActiveTools(), ...names])]);
				return {
					content: [{
						type: "text" as const,
						text: `Enabled ${name} MCP at ${endpoint.url} with ${names.length} tool(s): ${names.join(", ") || "none"}`,
					}],
					details: { action: "enable", server: name, endpoint: endpoint.url, tools: names },
				};
			} catch (error) {
				if (adopted) {
					deactivate(plannedNames);
					if (servers.get(name)?.client === connection.client) servers.delete(name);
				}
				await closeConnection(connection);
				throw error;
			}
		},
	});

	pi.on("session_shutdown", async () => {
		await Promise.all([...servers.keys()].map(disconnect));
	});
}

export default function mcpAdapter(pi: ExtensionAPI) {
	registerMcpAdapter(pi);
}
