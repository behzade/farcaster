import assert from "node:assert/strict";
import test from "node:test";
import { MCP_APPROVAL_SERVICE_KEY as ADAPTER_APPROVAL_SERVICE_KEY } from "../mcp-adapter/approval-service.ts";
import {
	MCP_APPROVAL_SERVICE_KEY,
	publishMcpApprovalService,
	unpublishMcpApprovalService,
} from "./mcp-approval-service.ts";

test("adapter and sandbox use the same global MCP approval service key", () => {
	assert.equal(MCP_APPROVAL_SERVICE_KEY, ADAPTER_APPROVAL_SERVICE_KEY);
});

test("MCP approval service cleanup removes only the owning registration", () => {
	const firstOwner = {};
	const secondOwner = {};
	const first = { version: 1 as const, owner: firstOwner, isEndpointApproved: () => false };
	const second = { version: 1 as const, owner: secondOwner, isEndpointApproved: () => true };
	publishMcpApprovalService(first);
	publishMcpApprovalService(second);
	unpublishMcpApprovalService(firstOwner);
	assert.equal(
		(globalThis as Record<symbol, unknown>)[MCP_APPROVAL_SERVICE_KEY],
		second,
	);
	unpublishMcpApprovalService(secondOwner);
	assert.equal(
		(globalThis as Record<symbol, unknown>)[MCP_APPROVAL_SERVICE_KEY],
		undefined,
	);
});
