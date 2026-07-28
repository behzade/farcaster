import { isIP } from "node:net";
import { domainToASCII } from "node:url";

export type McpProtocol = "http" | "https";

export interface McpEndpointInput {
	protocol?: McpProtocol;
	host: string;
	port: number;
	path: string;
}

export interface McpEndpoint {
	protocol: McpProtocol;
	host: string;
	port: number;
	path: string;
	url: string;
}

export function normalizeMcpHost(input: string): string {
	let value = input.trim();
	if (value.startsWith("[") && value.endsWith("]")) value = value.slice(1, -1);

	const ipVersion = isIP(value);
	if (ipVersion !== 0) return value.toLowerCase();
	if (
		value.length === 0 ||
		value.includes("://") ||
		/[\s\x00-\x1f\x7f/@?#:*]/.test(value)
	) {
		throw new Error("MCP host must be one exact hostname or IP without a scheme, credentials, port, path, query, fragment, or wildcard");
	}

	value = value.replace(/\.$/, "").toLowerCase();
	if (/^[0-9.]+$/.test(value)) throw new Error("Invalid MCP IP address");
	const ascii = domainToASCII(value);
	const labels = ascii.split(".");
	if (
		ascii.length === 0 ||
		ascii.length > 253 ||
		labels.some(
			(label) =>
				label.length === 0 ||
				label.length > 63 ||
				!/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/.test(label),
		)
	) {
		throw new Error("Invalid MCP hostname");
	}
	return ascii;
}

export function normalizeMcpEndpoint(input: McpEndpointInput): McpEndpoint {
	const protocol = input.protocol ?? "http";
	if (protocol !== "http" && protocol !== "https") {
		throw new Error("MCP protocol must be http or https");
	}
	if (!Number.isInteger(input.port) || input.port < 1 || input.port > 65_535) {
		throw new Error("MCP port must be an integer from 1 through 65535");
	}
	if (
		typeof input.path !== "string" ||
		!input.path.startsWith("/") ||
		input.path.startsWith("//") ||
		/[?#\\\s\x00-\x1f\x7f]/.test(input.path)
	) {
		throw new Error("MCP path must be one absolute URL path without whitespace, a query, or a fragment");
	}

	const host = normalizeMcpHost(input.host);
	const authority = isIP(host) === 6 ? `[${host}]` : host;
	const candidate = `${protocol}://${authority}:${input.port}${input.path}`;
	const parsed = new URL(candidate);
	if (parsed.username || parsed.password || parsed.search || parsed.hash) {
		throw new Error("MCP endpoint must not contain credentials, a query, or a fragment");
	}
	const path = parsed.pathname;
	const url = `${protocol}://${authority}:${input.port}${path}`;
	return { protocol, host, port: input.port, path, url };
}
