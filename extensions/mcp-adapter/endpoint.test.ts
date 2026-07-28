import assert from "node:assert/strict";
import test from "node:test";
import { normalizeMcpEndpoint } from "./endpoint.ts";

test("normalizes DNS, IPv4, IPv6, IDN, and the default protocol", () => {
	assert.deepEqual(
		normalizeMcpEndpoint({ host: "LOCALHOST.", port: 3845, path: "/mcp" }),
		{
			protocol: "http",
			host: "localhost",
			port: 3845,
			path: "/mcp",
			url: "http://localhost:3845/mcp",
		},
	);
	assert.equal(
		normalizeMcpEndpoint({ protocol: "https", host: "127.0.0.1", port: 443, path: "/mcp" }).url,
		"https://127.0.0.1:443/mcp",
	);
	assert.equal(
		normalizeMcpEndpoint({ host: "[::1]", port: 3845, path: "/mcp" }).url,
		"http://[::1]:3845/mcp",
	);
	assert.equal(
		normalizeMcpEndpoint({ host: "bücher.example", port: 443, path: "/mcp" }).host,
		"xn--bcher-kva.example",
	);
});

test("preserves an exact normalized path", () => {
	const endpoint = normalizeMcpEndpoint({
		protocol: "https",
		host: "example.com",
		port: 8443,
		path: "/team%20one/mcp",
	});
	assert.equal(endpoint.url, "https://example.com:8443/team%20one/mcp");
});

test("rejects ambiguous or unsafe endpoint components", () => {
	for (const input of [
		{ host: "https://example.com", port: 443, path: "/mcp" },
		{ host: "user@example.com", port: 443, path: "/mcp" },
		{ host: "example.com/path", port: 443, path: "/mcp" },
		{ host: "*.example.com", port: 443, path: "/mcp" },
		{ host: "999.1.1.1", port: 443, path: "/mcp" },
		{ host: "example.com", port: 0, path: "/mcp" },
		{ host: "example.com", port: 65_536, path: "/mcp" },
		{ host: "example.com", port: 443.5, path: "/mcp" },
		{ host: "example.com", port: 443, path: "mcp" },
		{ host: "example.com", port: 443, path: "//other/mcp" },
		{ host: "example.com", port: 443, path: "/mcp?token=x" },
		{ host: "example.com", port: 443, path: "/mcp#fragment" },
		{ host: "example.com", port: 443, path: "/mcp path" },
	] as const) {
		assert.throws(() => normalizeMcpEndpoint(input), JSON.stringify(input));
	}
});
